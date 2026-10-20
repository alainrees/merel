//! The text editor with Outlook's formatting bar, for a message and for an appointment's notes:
//! undo, paste, typeface and size, clear formatting, text colour, highlight, bold, italic,
//! underline, bullets and numbering, and under "⋯" strikethrough, indentation and alignment.
//!
//! egui has no rich-text editor, so this one is egui's own text field with a layouter that
//! draws the text the way a [`Rich`] says. The field edits the plain text; after every edit the
//! formatting follows the change (see [`Rich::edited`]), and what was typed takes the style of
//! the text before it or the one picked on the bar. Each paragraph is laid out on its own and
//! moved in for its indentation, list marker and alignment; the markers are painted in front.
//! Ctrl/⌘+B, I and U switch bold, italic and underline, Ctrl/⌘+Z and Ctrl/⌘+Y (or ⇧Z) undo and
//! redo text and formatting together.

use std::ops::Range;
use std::sync::Arc;

use egui::text::{ByteIndex, CCursor, CCursorRange, LayoutJob, LayoutSection, TextWrapping};
use egui::{Color32, FontId, Galley, Key, KeyboardShortcut, Modifiers, Response, RichText, Stroke, TextFormat, Ui, vec2};
use mailcraft_model::rich::{Align, DEFAULT_SIZE, Face, List, Para, Rgb, Rich, SIZES, Style, line_of, line_ranges};

use crate::Tokens;
use crate::helder::{self, Icon};

pub const UNDO: &str = "Ongedaan maken";
pub const REDO: &str = "Opnieuw";
pub const UNDO_MORE: &str = "Meer ongedaan maken";
pub const PASTE: &str = "Plakken";
pub const FACE: &str = "Lettertype";
pub const SIZE: &str = "Tekengrootte";
pub const CLEAR: &str = "Opmaak wissen";
pub const COLOR: &str = "Tekstkleur";
pub const COLOR_MENU: &str = "Tekstkleur kiezen";
pub const HIGHLIGHT: &str = "Markeren";
pub const HIGHLIGHT_MENU: &str = "Markeerkleur kiezen";
pub const BOLD: &str = "Vet";
pub const ITALIC: &str = "Cursief";
pub const UNDERLINE: &str = "Onderstrepen";
pub const STRIKE: &str = "Doorhalen";
pub const BULLETS: &str = "Opsommingstekens";
pub const NUMBERING: &str = "Nummering";
pub const MORE: &str = "Meer opmaakopties";
pub const INDENT_MORE: &str = "Inspringing vergroten";
pub const INDENT_LESS: &str = "Inspringing verkleinen";
pub const ALIGN_LEFT: &str = "Links uitlijnen";
pub const ALIGN_CENTER: &str = "Centreren";
pub const ALIGN_RIGHT: &str = "Rechts uitlijnen";
pub const AUTOMATIC: &str = "Automatisch";
pub const NO_HIGHLIGHT: &str = "Geen markering";

/// Outlook's standard text colours.
pub const TEXT_COLORS: [(&str, Rgb); 10] = [
    ("Donkerrood", Rgb(0xC0, 0x00, 0x00)),
    ("Rood", Rgb(0xFF, 0x00, 0x00)),
    ("Oranje", Rgb(0xFF, 0xC0, 0x00)),
    ("Geel", Rgb(0xFF, 0xFF, 0x00)),
    ("Lichtgroen", Rgb(0x92, 0xD0, 0x50)),
    ("Groen", Rgb(0x00, 0xB0, 0x50)),
    ("Lichtblauw", Rgb(0x00, 0xB0, 0xF0)),
    ("Blauw", Rgb(0x00, 0x70, 0xC0)),
    ("Donkerblauw", Rgb(0x00, 0x20, 0x60)),
    ("Paars", Rgb(0x70, 0x30, 0xA0)),
];

/// The marker colours; light enough for dark text on top.
pub const HIGHLIGHTS: [(&str, Rgb); 5] = [
    ("Geel markeren", Rgb(0xFF, 0xF1, 0x00)),
    ("Groen markeren", Rgb(0x7F, 0xFF, 0x7F)),
    ("Turkoois markeren", Rgb(0x7F, 0xFF, 0xFF)),
    ("Roze markeren", Rgb(0xFF, 0xA8, 0xE0)),
    ("Grijs markeren", Rgb(0xD9, 0xD9, 0xD9)),
];

/// Pixels per indentation step, and the room a list marker takes.
const INDENT: f32 = 32.0;
const MARKER: f32 = 24.0;
/// Undo steps kept.
const MAX_UNDO: usize = 200;
/// Typing within this many seconds of the last change is one undo step.
const UNDO_PAUSE: f64 = 1.0;

/// The text field's size in pixels for a size in points: 12 pt is the app's field text.
fn px(points: u8) -> f32 {
    f32::from(points) * helder::FIELD_TEXT / f32::from(DEFAULT_SIZE)
}

fn color(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// What the text and its formatting looked like, for undo.
#[derive(Clone)]
struct Snapshot {
    text: String,
    rich: Rich,
    cursor: Range<usize>,
}

/// The editor's state next to its text: the formatting and what the editor remembers between
/// frames.
#[derive(Clone)]
pub struct Editor {
    pub rich: Rich,
    id: egui::Id,
    /// The text as the formatting last followed it; a difference means it was changed from
    /// outside (a template dropped in) and the formatting catches up.
    seen: String,
    /// The selection as last seen, as character positions; the bar acts on it.
    selection: Range<usize>,
    /// A style picked on the bar with nothing selected: the next characters typed at `.0` take it.
    pending: Option<(usize, Style)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_change: f64,
}

/// What the bar asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Bold,
    Italic,
    Underline,
    Strike,
    Face(Face),
    Size(u8),
    Color(Option<Rgb>),
    Highlight(Option<Rgb>),
    Clear,
    List(List),
    Indent(bool),
    Align(Align),
    Undo,
    Redo,
    Paste,
}

impl Editor {
    /// An editor over `text` with `rich` formatting, or plain when there is none. `salt` makes
    /// its text field's id.
    pub fn new(salt: &str, text: &str, rich: Option<Rich>) -> Self {
        let mut rich = rich.unwrap_or_default();
        rich.fit(text);
        Editor {
            rich,
            id: egui::Id::new(("rich-edit", salt)),
            seen: text.to_string(),
            selection: 0..0,
            pending: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_change: f64::NEG_INFINITY,
        }
    }

    /// The text field's id.
    pub fn id(&self) -> egui::Id {
        self.id
    }

    /// The formatting over `text`, caught up with changes made from outside the editor.
    pub fn rich_for(&self, text: &str) -> Rich {
        let mut r = self.rich.clone();
        if self.seen != text {
            r.edited(&self.seen, text, Style::default());
        }
        r
    }

    /// The formatting as it should be stored: `None` when nothing is formatted.
    pub fn stored(&self, text: &str) -> Option<Rich> {
        Some(self.rich_for(text)).filter(|r| !r.is_plain())
    }

    /// Follows a change made from outside.
    fn catch_up(&mut self, text: &str) {
        if self.seen != text {
            self.rich = self.rich_for(text);
            self.seen = text.to_string();
        }
    }

    /// The style the bar shows and typing takes: the one picked, else that of the selection's
    /// first character, else that of the character before the cursor.
    fn current(&self) -> Style {
        match self.pending {
            Some((at, style)) if self.selection == (at..at) => style,
            _ if !self.selection.is_empty() => self.rich.style_of(self.selection.start),
            _ => self.rich.style_at(self.selection.start),
        }
    }

    /// The paragraph the cursor is in.
    fn para(&self, text: &str) -> Para {
        self.rich.paras.get(line_of(text, self.selection.start)).copied().unwrap_or_default()
    }

    fn snapshot(&self, text: &str) -> Snapshot {
        Snapshot { text: text.to_string(), rich: self.rich.clone(), cursor: self.selection.clone() }
    }

    fn remember(&mut self, text: &str) {
        self.undo.push(self.snapshot(text));
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Puts `s` back, keeping the present on `other` so it can be put back in turn.
    fn restore(&mut self, ctx: &egui::Context, text: &mut String, s: Snapshot, redo: bool) {
        let now = self.snapshot(text);
        if redo {
            self.undo.push(now)
        } else {
            self.redo.push(now)
        }
        *text = s.text;
        self.rich = s.rich;
        self.seen = text.clone();
        self.selection = s.cursor;
        self.pending = None;
        self.last_change = f64::NEG_INFINITY;
        self.select(ctx);
    }

    /// Gives the field the focus with the remembered selection, after a click on the bar.
    fn select(&self, ctx: &egui::Context) {
        let mut state = egui::TextEdit::load_state(ctx, self.id).unwrap_or_default();
        let range = CCursorRange::two(CCursor::new(self.selection.start), CCursor::new(self.selection.end));
        state.cursor.set_char_range(Some(range));
        state.store(ctx, self.id);
        ctx.memory_mut(|m| m.request_focus(self.id));
    }

    /// Changes the style of the selection, or of what will be typed when nothing is selected.
    fn restyle(&mut self, text: &str, f: impl Fn(&mut Style)) {
        if self.selection.is_empty() {
            let mut s = self.current();
            f(&mut s);
            self.pending = Some((self.selection.start, s));
        } else {
            self.remember(text);
            self.rich.apply(self.selection.clone(), f);
        }
    }

    /// The lines the selection touches.
    fn lines(&self, text: &str) -> Range<usize> {
        line_of(text, self.selection.start)..line_of(text, self.selection.end) + 1
    }

    fn run(&mut self, ctx: &egui::Context, text: &mut String, command: Command) {
        self.catch_up(text);
        let current = self.current();
        match command {
            Command::Bold => self.restyle(text, |s| s.bold = !current.bold),
            Command::Italic => self.restyle(text, |s| s.italic = !current.italic),
            Command::Underline => self.restyle(text, |s| s.underline = !current.underline),
            Command::Strike => self.restyle(text, |s| s.strike = !current.strike),
            Command::Face(face) => self.restyle(text, |s| s.face = face),
            Command::Size(size) => self.restyle(text, |s| s.size = Some(size)),
            Command::Color(c) => self.restyle(text, |s| s.color = c),
            Command::Highlight(c) => self.restyle(text, |s| s.highlight = c),
            Command::Clear => {
                self.restyle(text, |s| *s = Style::default());
                let lines = self.lines(text);
                self.rich.apply_paras(lines, |p| *p = Para::default());
            }
            Command::List(kind) => {
                self.remember(text);
                let lines = self.lines(text);
                let all =
                    self.rich.paras[lines.start.min(self.rich.paras.len())..lines.end.min(self.rich.paras.len())].iter().all(|p| p.list == kind);
                self.rich.apply_paras(lines, |p| p.list = if all { List::None } else { kind });
            }
            Command::Indent(more) => {
                self.remember(text);
                let lines = self.lines(text);
                self.rich.apply_paras(lines, |p| p.indent = if more { p.indent.saturating_add(1) } else { p.indent.saturating_sub(1) });
            }
            Command::Align(align) => {
                self.remember(text);
                let lines = self.lines(text);
                self.rich.apply_paras(lines, |p| p.align = align);
            }
            Command::Undo => {
                if let Some(s) = self.undo.pop() {
                    self.restore(ctx, text, s, false);
                }
                return;
            }
            Command::Redo => {
                if let Some(s) = self.redo.pop() {
                    self.restore(ctx, text, s, true);
                }
                return;
            }
            Command::Paste => {
                self.select(ctx);
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                return;
            }
        }
        self.select(ctx);
    }
}

/// Whether the editor's typefaces are loaded; they come in a frame after the app sets its
/// fonts, and until then text is drawn in the plain font.
fn loaded(ctx: &egui::Context) -> bool {
    let (family, _) = crate::fonts::family(Face::Sans, false, false);
    ctx.fonts(|f| f.definitions().families.contains_key(&family))
}

/// The family `face` is drawn in, and whether to slant it, falling back to the plain font.
fn family(loaded: bool, face: Face, bold: bool, italic: bool) -> (egui::FontFamily, bool) {
    if loaded { crate::fonts::family(face, bold, italic) } else { (egui::FontFamily::Proportional, italic) }
}

/// `c` as the editor shows it: lightened on a dark card until it reads at 4.5:1, the way
/// Outlook adapts colours in dark mode. The message keeps the colour as chosen.
fn readable(t: &Tokens, c: Color32) -> Color32 {
    if !t.dark {
        return c;
    }
    let mut shown = c;
    for step in 1..=10 {
        if crate::theme::contrast(shown, t.card) >= 4.5 {
            break;
        }
        shown = c.lerp_to_gamma(Color32::WHITE, step as f32 / 10.0);
    }
    shown
}

/// How `s` is drawn.
fn format(t: &Tokens, s: &Style, loaded: bool) -> TextFormat {
    let (family, slant) = family(loaded, s.face, s.bold, s.italic);
    let ink = match (s.color, s.highlight) {
        (Some(c), Some(_)) => color(c),
        (Some(c), None) => readable(t, color(c)),
        // Dark text on a marker, whatever the theme.
        (None, Some(_)) => Color32::from_rgb(0x1F, 0x1F, 0x1F),
        (None, None) => t.text,
    };
    TextFormat {
        font_id: FontId::new(px(s.points()), family),
        color: ink,
        background: s.highlight.map(color).unwrap_or(Color32::TRANSPARENT),
        italics: slant,
        underline: if s.underline { Stroke::new(1.0, ink) } else { Stroke::NONE },
        strikethrough: if s.strike { Stroke::new(1.0, ink) } else { Stroke::NONE },
        ..TextFormat::default()
    }
}

/// How far paragraph `p`'s lines move in from the left edge.
fn inset(p: &Para) -> f32 {
    f32::from(p.indent) * INDENT + if p.list == List::None { 0.0 } else { MARKER }
}

/// The byte offset of each character of `text`, and its length at the end.
fn byte_offsets(text: &str) -> Vec<usize> {
    text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).collect()
}

/// Lays `text` out the way `rich` says, within `wrap` points: each paragraph on its own,
/// wrapped to the room its indentation leaves, then moved in and aligned.
fn layout(ui: &Ui, t: &Tokens, text: &str, rich: &Rich, wrap: f32) -> Arc<Galley> {
    let loaded = loaded(ui.ctx());
    let bytes = byte_offsets(text);
    let mut full = LayoutJob { text: text.to_string(), wrap: TextWrapping { max_width: wrap, ..Default::default() }, ..Default::default() };
    full.keep_trailing_whitespace = true;
    let mut start = 0;
    for run in &rich.runs {
        let end = start + run.len;
        full.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: ByteIndex(bytes[start])..ByteIndex(bytes[end]),
            format: format(t, &run.style, loaded),
        });
        start = end;
    }
    if full.sections.is_empty() {
        full.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: ByteIndex(0)..ByteIndex(0),
            format: format(t, &rich.style_at(0), loaded),
        });
    }
    let mut galleys = Vec::new();
    let mut shifts = Vec::new();
    for (i, range) in line_ranges(text).into_iter().enumerate() {
        let p = rich.paras.get(i).copied().unwrap_or_default();
        let room = (wrap - inset(&p)).max(40.0);
        let (from, to) = (bytes[range.start], bytes[range.end]);
        let mut job =
            LayoutJob { text: text[from..to].to_string(), wrap: TextWrapping { max_width: room, ..Default::default() }, ..Default::default() };
        job.keep_trailing_whitespace = true;
        let mut start = 0;
        for run in &rich.runs {
            let end = start + run.len;
            let (a, b) = (start.max(range.start), end.min(range.end));
            if a < b {
                job.sections.push(LayoutSection {
                    leading_space: 0.0,
                    byte_range: ByteIndex(bytes[a] - from)..ByteIndex(bytes[b] - from),
                    format: format(t, &run.style, loaded),
                });
            }
            start = end;
        }
        if job.sections.is_empty() {
            // An empty line still has the height of the text it would hold.
            job.sections.push(LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(0)..ByteIndex(0),
                format: format(t, &rich.style_at(range.start), loaded),
            });
        }
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        shifts.push((galley.rows.len(), inset(&p), room, p.align));
        galleys.push(galley);
    }
    let mut merged = Galley::concat(Arc::new(full), &galleys, ui.ctx().pixels_per_point());
    let mut row = 0;
    for (rows, inset, room, align) in shifts {
        for placed in &mut merged.rows[row..row + rows] {
            let free = (room - placed.size.x).max(0.0);
            placed.pos.x += inset
                + match align {
                    Align::Left => 0.0,
                    Align::Center => (free / 2.0).round(),
                    Align::Right => free.floor(),
                };
        }
        row += rows;
    }
    if row > 0 {
        let first = merged.rows[0].rect();
        merged.rect = merged.rows.iter().fold(first, |r, p| r.union(p.rect()));
        merged.mesh_bounds = merged.rows.iter().fold(egui::Rect::NOTHING, |r, p| r.union(p.visuals.mesh_bounds.translate(p.pos.to_vec2())));
    }
    Arc::new(merged)
}

/// The text field. `label` is its name for a screen reader, `hint` the grey text it shows while
/// empty; `min_height` the least it takes.
pub fn edit(ui: &mut Ui, t: &Tokens, ed: &mut Editor, text: &mut String, label: &str, hint: &str, min_height: f32) -> Response {
    ed.catch_up(text);
    let ctx = ui.ctx().clone();
    // The editor's own keys, before the field sees them: its undo would not know the formatting.
    if ctx.memory(|m| m.has_focus(ed.id)) {
        let key = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
        let shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if key(shift, Key::Z) || key(Modifiers::COMMAND, Key::Y) {
            ed.run(&ctx, text, Command::Redo);
        } else if key(Modifiers::COMMAND, Key::Z) {
            ed.run(&ctx, text, Command::Undo);
        }
        if key(shift, Key::L) {
            ed.run(&ctx, text, Command::List(List::Bullet));
        }
        for (k, c) in [(Key::B, Command::Bold), (Key::I, Command::Italic), (Key::U, Command::Underline)] {
            if key(Modifiers::COMMAND, k) {
                ed.run(&ctx, text, c);
            }
        }
    }
    // The style what is typed now takes.
    if let Some(range) = egui::TextEdit::load_state(&ctx, ed.id).and_then(|s| s.cursor.char_range())
        && ctx.memory(|m| m.has_focus(ed.id))
    {
        let (a, b) = (range.primary.index.0.min(range.secondary.index.0), range.primary.index.0.max(range.secondary.index.0));
        ed.selection = a..b;
    }
    let typing = ed.current();
    let cursor = Some(ed.selection.start);
    let before = text.clone();
    let rich = ed.rich.clone();
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let s = buf.as_str();
        if s == before {
            layout(ui, t, s, &rich, wrap)
        } else {
            let mut r = rich.clone();
            r.edited_at(&before, s, typing, cursor);
            layout(ui, t, s, &r, wrap)
        }
    };
    let output = egui::TextEdit::multiline(text)
        .id(ed.id)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::symmetric(4, 8))
        .desired_width(f32::INFINITY)
        .min_size(vec2(0.0, min_height))
        .hint_text(RichText::new(hint).size(helder::FIELD_TEXT).color(t.text_dim))
        .layouter(&mut layouter)
        .show(ui);
    if *text != before {
        let now = ctx.input(|i| i.time);
        if now - ed.last_change > UNDO_PAUSE || before.chars().count().abs_diff(text.chars().count()) > 1 {
            let snap = Snapshot { text: before.clone(), rich: ed.rich.clone(), cursor: ed.selection.clone() };
            ed.undo.push(snap);
            if ed.undo.len() > MAX_UNDO {
                ed.undo.remove(0);
            }
            ed.redo.clear();
        }
        ed.last_change = now;
        ed.rich.edited_at(&before, text, typing, cursor);
        ed.seen = text.clone();
        ed.pending = None;
    }
    if let Some(range) = output.cursor_range {
        let (a, b) = (range.primary.index.0.min(range.secondary.index.0), range.primary.index.0.max(range.secondary.index.0));
        if ed.pending.is_some_and(|(at, _)| at != a || a != b) {
            ed.pending = None;
        }
        ed.selection = a..b;
    }
    // The list markers, in front of the first row of their lines.
    let galley = &output.galley;
    let mut start = 0;
    for (i, range) in line_ranges(text).into_iter().enumerate() {
        if let Some(marker) = ed.rich.marker(i) {
            let p = ed.rich.paras.get(i).copied().unwrap_or_default();
            let row = galley.pos_from_cursor(CCursor::new(range.start));
            let style = ed.rich.style_of(range.start.max(start));
            let f = format(t, &Style { bold: false, italic: false, underline: false, strike: false, highlight: None, ..style }, loaded(&ctx));
            let pos = output.galley_pos + vec2(f32::from(p.indent) * INDENT + 4.0, row.center().y);
            ui.painter().text(pos, egui::Align2::LEFT_CENTER, marker, f.font_id, f.color);
        }
        start = range.end + 1;
    }
    let r = output.response.response;
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    r
}

/// A square button on the bar with only an icon, lit while `on`; `label` is its tooltip and
/// its name for a screen reader.
fn bar_button(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, on: bool) -> Response {
    let color = if on { t.accent_text } else { t.text };
    let r = ui.add(
        egui::Button::image(icon.image(helder::ICON, color))
            .frame_when_inactive(on)
            .selected(on)
            .corner_radius(helder::CONTROL_RADIUS)
            .min_size(vec2(32.0, 32.0)),
    );
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, label));
    r.on_hover_text(label)
}

/// A narrow chevron that opens a menu next to a bar button.
fn chevron<R>(ui: &mut Ui, t: &Tokens, label: &str, add: impl FnOnce(&mut Ui) -> R) {
    let button = egui::Button::image(Icon::ChevronDown.image(14.0, t.text_dim)).frame_when_inactive(false).min_size(vec2(18.0, 32.0));
    ui.add_space(-ui.spacing().item_spacing.x + 1.0);
    let (r, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, add);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let _ = r.on_hover_text(label);
}

/// A grey rounded field that opens a list, as the typeface and size on Outlook's bar.
fn pill<R>(ui: &mut Ui, t: &Tokens, text: &str, width: f32, label: &str, add: impl FnOnce(&mut Ui) -> R) {
    let button = egui::Button::new(RichText::new(text).size(14.0).color(t.text))
        .fill(t.window)
        .stroke(Stroke::NONE)
        .corner_radius(helder::CONTROL_RADIUS)
        .min_size(vec2(width, 32.0));
    let (r, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, add);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
    let _ = r.on_hover_text(label);
}

/// A colour well in a menu: a square of `c` with its name as tooltip and label.
fn swatch(ui: &mut Ui, t: &Tokens, name: &str, c: Rgb) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
    ui.painter().rect(rect, 4, color(c), Stroke::new(1.0, if r.hovered() { t.text } else { t.card_border }), egui::StrokeKind::Inside);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    r.on_hover_text(name).clicked()
}

/// The formatting bar over the text field.
pub fn bar(ui: &mut Ui, t: &Tokens, ed: &mut Editor, text: &mut String) {
    ed.catch_up(text);
    let current = ed.current();
    let para = ed.para(text);
    let mut command = None;
    helder::quiet(ui, t, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.add_enabled_ui(!ed.undo.is_empty(), |ui| {
                if bar_button(ui, t, Icon::Undo, UNDO, false).clicked() {
                    command = Some(Command::Undo);
                }
            });
            chevron(ui, t, UNDO_MORE, |ui| {
                if ui.add_enabled(!ed.redo.is_empty(), egui::Button::image_and_text(Icon::Redo.image(16.0, t.text), REDO)).clicked() {
                    command = Some(Command::Redo);
                    ui.close();
                }
            });
            if bar_button(ui, t, Icon::Paste, PASTE, false).clicked() {
                command = Some(Command::Paste);
            }
            helder::bar_separator(ui, t);
            pill(ui, t, current.face.name(), 150.0, FACE, |ui| {
                let loaded = loaded(ui.ctx());
                for face in Face::ALL {
                    let (family, _) = family(loaded, face, false, false);
                    let name = RichText::new(face.name()).font(FontId::new(15.0, family));
                    if ui.selectable_label(current.face == face, name).clicked() {
                        command = Some(Command::Face(face));
                        ui.close();
                    }
                }
            });
            pill(ui, t, &current.points().to_string(), 40.0, SIZE, |ui| {
                for size in SIZES {
                    if ui.selectable_label(current.points() == size, size.to_string()).clicked() {
                        command = Some(Command::Size(size));
                        ui.close();
                    }
                }
            });
            if bar_button(ui, t, Icon::ClearFormat, CLEAR, false).clicked() {
                command = Some(Command::Clear);
            }
            // Text colour: the button puts on the last colour picked, shown in the bar under the A.
            let ink = ui.ctx().data(|d| d.get_temp::<Rgb>(ed.id.with("ink"))).unwrap_or(TEXT_COLORS[1].1);
            let r = bar_button(ui, t, Icon::TextColor, COLOR, false);
            ui.painter().rect_filled(egui::Rect::from_center_size(r.rect.center_bottom() - vec2(0.0, 6.0), vec2(16.0, 3.0)), 1, color(ink));
            if r.clicked() {
                command = Some(Command::Color(Some(ink)));
            }
            chevron(ui, t, COLOR_MENU, |ui| {
                if ui.button(AUTOMATIC).clicked() {
                    command = Some(Command::Color(None));
                    ui.close();
                }
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(140.0);
                    for (name, c) in TEXT_COLORS {
                        if swatch(ui, t, name, c) {
                            ui.ctx().data_mut(|d| d.insert_temp(ed.id.with("ink"), c));
                            command = Some(Command::Color(Some(c)));
                            ui.close();
                        }
                    }
                });
            });
            let marker = ui.ctx().data(|d| d.get_temp::<Rgb>(ed.id.with("marker"))).unwrap_or(HIGHLIGHTS[0].1);
            let r = bar_button(ui, t, Icon::Highlighter, HIGHLIGHT, current.highlight.is_some());
            ui.painter().rect_filled(egui::Rect::from_center_size(r.rect.center_bottom() - vec2(0.0, 6.0), vec2(16.0, 3.0)), 1, color(marker));
            if r.clicked() {
                command = Some(Command::Highlight(if current.highlight.is_some() { None } else { Some(marker) }));
            }
            chevron(ui, t, HIGHLIGHT_MENU, |ui| {
                if ui.button(NO_HIGHLIGHT).clicked() {
                    command = Some(Command::Highlight(None));
                    ui.close();
                }
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(140.0);
                    for (name, c) in HIGHLIGHTS {
                        if swatch(ui, t, name, c) {
                            ui.ctx().data_mut(|d| d.insert_temp(ed.id.with("marker"), c));
                            command = Some(Command::Highlight(Some(c)));
                            ui.close();
                        }
                    }
                });
            });
            for (icon, label, on, c) in [
                (Icon::Bold, BOLD, current.bold, Command::Bold),
                (Icon::Italic, ITALIC, current.italic, Command::Italic),
                (Icon::Underline, UNDERLINE, current.underline, Command::Underline),
            ] {
                if bar_button(ui, t, icon, label, on).clicked() {
                    command = Some(c);
                }
            }
            helder::bar_separator(ui, t);
            if bar_button(ui, t, Icon::BulletList, BULLETS, para.list == List::Bullet).clicked() {
                command = Some(Command::List(List::Bullet));
            }
            if bar_button(ui, t, Icon::NumberList, NUMBERING, para.list == List::Number).clicked() {
                command = Some(Command::List(List::Number));
            }
            let more = egui::Button::image(Icon::Ellipsis.image(helder::ICON, t.text)).frame_when_inactive(false).min_size(vec2(32.0, 32.0));
            let (r, _) = egui::containers::menu::MenuButton::from_button(more).ui(ui, |ui| {
                for (icon, label, on, c) in [
                    (Icon::Strikethrough, STRIKE, current.strike, Command::Strike),
                    (Icon::IndentMore, INDENT_MORE, false, Command::Indent(true)),
                    (Icon::IndentLess, INDENT_LESS, false, Command::Indent(false)),
                    (Icon::AlignLeft, ALIGN_LEFT, para.align == Align::Left, Command::Align(Align::Left)),
                    (Icon::AlignCenter, ALIGN_CENTER, para.align == Align::Center, Command::Align(Align::Center)),
                    (Icon::AlignRight, ALIGN_RIGHT, para.align == Align::Right, Command::Align(Align::Right)),
                ] {
                    let color = if on { t.accent_text } else { t.text };
                    let b = egui::Button::image_and_text(icon.image(16.0, color), RichText::new(label).color(color)).selected(on);
                    if ui.add(b).clicked() {
                        command = Some(c);
                        ui.close();
                    }
                }
            });
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, MORE));
            let _ = r.on_hover_text(MORE);
        });
    });
    if let Some(c) = command {
        let ctx = ui.ctx().clone();
        ed.run(&ctx, text, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    struct State {
        text: String,
        ed: Editor,
    }

    fn harness(text: &str) -> Harness<'static, State> {
        let state = State { text: text.into(), ed: Editor::new("test", text, None) };
        Harness::builder().with_size(vec2(900.0, 500.0)).build_ui_state(
            |ui, s: &mut State| {
                ui.ctx().set_fonts(crate::fonts::definitions());
                let t = Tokens::light();
                bar(ui, &t, &mut s.ed, &mut s.text);
                edit(ui, &t, &mut s.ed, &mut s.text, "Bericht", "", 200.0);
            },
            state,
        )
    }

    /// Selects `range` in the field, as dragging over it would.
    fn select(h: &mut Harness<'static, State>, range: Range<usize>) {
        h.state_mut().ed.selection = range.clone();
        let id = h.state().ed.id;
        let mut state = egui::TextEdit::load_state(&h.ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end))));
        state.store(&h.ctx, id);
    }

    #[test]
    fn the_bar_makes_the_selection_bold_and_back_and_undo_takes_it_away() {
        let mut h = harness("Hallo wereld");
        h.run();
        select(&mut h, 0..5);
        h.get_by_label(BOLD).click();
        h.run();
        assert!(h.state().ed.rich.style_of(0).bold && !h.state().ed.rich.style_of(6).bold);
        assert!(h.state().ed.current().bold, "the bar shows it is on");
        h.get_by_label(BOLD).click();
        h.run();
        assert!(h.state().ed.rich.is_plain(), "a second click takes it off");
        h.get_by_label(UNDO).click();
        h.run();
        assert!(h.state().ed.rich.style_of(0).bold, "undo puts the bold back");
        h.get_by_label(UNDO_MORE).click();
        h.run();
        h.get_by_label(REDO).click();
        h.run();
        assert!(h.state().ed.rich.is_plain(), "and redo takes it off again");
    }

    #[test]
    fn colour_size_face_and_highlight_go_on_the_selection() {
        let mut h = harness("Rood en groot");
        h.run();
        select(&mut h, 0..4);
        h.get_by_label(COLOR_MENU).click();
        h.run();
        h.get_by_label("Groen").click();
        h.run();
        h.get_by_label(SIZE).click();
        h.run();
        h.get_by_label("18").click();
        h.run();
        h.get_by_label(FACE).click();
        h.run();
        h.get_by_label("DejaVu Serif").click();
        h.run();
        h.get_by_label(HIGHLIGHT).click();
        h.run();
        let s = h.state().ed.rich.style_of(0);
        assert_eq!((s.color, s.size, s.face, s.highlight), (Some(Rgb(0x00, 0xB0, 0x50)), Some(18), Face::Serif, Some(HIGHLIGHTS[0].1)));
        assert_eq!(h.state().ed.rich.style_of(5), Style::default());
        h.get_by_label(CLEAR).click();
        h.run();
        assert!(h.state().ed.rich.is_plain());
    }

    #[test]
    fn lists_indentation_and_alignment_go_on_the_lines_of_the_selection() {
        let mut h = harness("een\ntwee\ndrie");
        h.run();
        select(&mut h, 1..6);
        h.get_by_label(NUMBERING).click();
        h.run();
        let lists: Vec<List> = h.state().ed.rich.paras.iter().map(|p| p.list).collect();
        assert_eq!(lists, [List::Number, List::Number, List::None]);
        h.get_by_label(MORE).click();
        h.run();
        h.get_by_label(INDENT_MORE).click();
        h.run();
        h.get_by_label(MORE).click();
        h.run();
        h.get_by_label(ALIGN_CENTER).click();
        h.run();
        let p = h.state().ed.rich.paras[1];
        assert_eq!((p.indent, p.align), (1, Align::Center));
        h.get_by_label(NUMBERING).click();
        h.run();
        assert!(h.state().ed.rich.paras.iter().all(|p| p.list == List::None), "a second click ends the list");
    }

    #[test]
    fn typing_after_a_style_is_picked_takes_it_and_text_from_outside_is_plain() {
        let mut h = harness("Hoi ");
        h.run();
        select(&mut h, 4..4);
        h.get_by_label(ITALIC).click();
        h.run();
        h.state_mut().ed.selection = 4..4;
        h.state_mut().ed.pending = Some((4, Style { italic: true, ..Style::default() }));
        h.get_by_label("Bericht").focus();
        h.run();
        h.get_by_label("Bericht").type_text("Sanne");
        h.run();
        assert!(h.state().text.ends_with("Sanne"), "{}", h.state().text);
        let r = &h.state().ed.rich;
        assert!(!r.style_of(0).italic && r.style_of(4).italic && r.style_of(8).italic, "{:?}", r.runs);
        h.state_mut().text.insert_str(0, "Template ");
        h.run();
        let r = h.state().ed.rich_for(&h.state().text);
        assert!(!r.style_of(0).italic && r.style_of(13).italic, "{:?}", r.runs);
    }

    #[test]
    fn every_text_colour_reads_on_a_dark_card() {
        let t = Tokens::dark();
        for (name, c) in TEXT_COLORS {
            assert!(crate::theme::contrast(readable(&t, color(c)), t.card) >= 4.5, "{name}");
        }
        assert_eq!(readable(&Tokens::light(), color(TEXT_COLORS[8].1)), color(TEXT_COLORS[8].1), "light mode shows the colour as chosen");
    }

    #[test]
    fn the_editor_lays_lines_out_with_their_indent_and_alignment() {
        let mut h = harness("links\nmidden");
        h.run();
        select(&mut h, 7..7);
        h.get_by_label(MORE).click();
        h.run();
        h.get_by_label(ALIGN_RIGHT).click();
        h.run();
        select(&mut h, 0..0);
        h.get_by_label(BULLETS).click();
        h.run();
        let rich = h.state().ed.rich.clone();
        let out = std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = out.clone();
        let mut l = Harness::new_ui(move |ui| {
            ui.ctx().set_fonts(crate::fonts::definitions());
            if let Ok(mut o) = sink.lock() {
                *o = Some(layout(ui, &Tokens::light(), "links\nmidden", &rich, 400.0));
            }
        });
        l.run();
        let galley = out.lock().ok().and_then(|o| o.clone()).expect("laid out");
        assert_eq!(galley.rows.len(), 2);
        assert!((galley.rows[0].pos.x - MARKER).abs() < 0.5, "the bullet line moves in for its marker: {}", galley.rows[0].pos.x);
        assert!(galley.rows[1].rect().right() > 395.0, "the right-aligned line ends at the margin: {:?}", galley.rows[1].rect());
        assert_eq!(galley.text(), "links\nmidden");
    }
}
