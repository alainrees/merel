//! The Helder style: a light grey window ground with white rounded cards, quiet buttons that
//! are a line icon and a label, one blue main button per bar, fields with only an underline,
//! and switches instead of tick boxes. The pieces here are what the windows build with, so
//! every window gets the same look; the colours come from [`Tokens`], so both themes do.
//!
//! The icons are Lucide's (ISC licence, `assets/icons/LICENSE-Lucide.txt`), drawn white so a
//! tint gives them any colour, and embedded in the binary.

use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Image, InnerResponse, Layout, Margin, Rect, Response, RichText, Sense, Stroke, TextStyle, Ui,
    WidgetInfo, WidgetText, WidgetType, pos2, text::LayoutJob, vec2,
};

use crate::Tokens;

/// The side of an icon in a button or a form row.
pub const ICON: f32 = 18.0;
/// The height of a button in a command bar.
pub const BUTTON_HEIGHT: f32 = 36.0;
/// The corners of a card, and of a button or a field.
pub const CARD_RADIUS: u8 = 12;
pub const CONTROL_RADIUS: u8 = 8;
/// The width of the icon column at the start of a form row.
pub const ROW_ICON: f32 = 40.0;
/// The size of text in a field, and of the large one that names the item.
pub const FIELD_TEXT: f32 = 15.0;
pub const TITLE_TEXT: f32 = 20.0;

/// The line icons the app draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Bell,
    Calendar,
    CalendarX,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Clock,
    Ellipsis,
    Forward,
    Lock,
    MapPin,
    Notes,
    Repeat,
    Save,
    Send,
    Trash,
    Type,
    Users,
    X,
}

impl Icon {
    /// Every icon, for the test that each one loads.
    pub const ALL: [Icon; 19] = [
        Icon::Bell,
        Icon::Calendar,
        Icon::CalendarX,
        Icon::ChevronDown,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::Clock,
        Icon::Ellipsis,
        Icon::Forward,
        Icon::Lock,
        Icon::MapPin,
        Icon::Notes,
        Icon::Repeat,
        Icon::Save,
        Icon::Send,
        Icon::Trash,
        Icon::Type,
        Icon::Users,
        Icon::X,
    ];

    /// The image's uri and its SVG source.
    pub fn source(self) -> (&'static str, &'static [u8]) {
        macro_rules! svg {
            ($name:literal) => {
                (concat!("bytes://icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))
            };
        }
        match self {
            Icon::Bell => svg!("bell"),
            Icon::Calendar => svg!("calendar"),
            Icon::CalendarX => svg!("calendar-x"),
            Icon::ChevronDown => svg!("chevron-down"),
            Icon::ChevronLeft => svg!("chevron-left"),
            Icon::ChevronRight => svg!("chevron-right"),
            Icon::Clock => svg!("clock"),
            Icon::Ellipsis => svg!("ellipsis"),
            Icon::Forward => svg!("forward"),
            Icon::Lock => svg!("lock"),
            Icon::MapPin => svg!("map-pin"),
            Icon::Notes => svg!("notes"),
            Icon::Repeat => svg!("repeat"),
            Icon::Save => svg!("save"),
            Icon::Send => svg!("send"),
            Icon::Trash => svg!("trash"),
            Icon::Type => svg!("type"),
            Icon::Users => svg!("users"),
            Icon::X => svg!("x"),
        }
    }

    /// The icon `size` points square in `color`.
    pub fn image(self, size: f32, color: Color32) -> Image<'static> {
        let (uri, bytes) = self.source();
        Image::from_bytes(uri, bytes).fit_to_exact_size(vec2(size, size)).tint(color)
    }
}

/// Runs `add` with buttons that show no frame until the pointer is on them, then a soft fill
/// without a border, as the command bars and menus of the Helder style do.
pub fn quiet<R>(ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        for w in [&mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.bg_stroke = Stroke::NONE;
            w.expansion = 0.0;
        }
        v.widgets.hovered.weak_bg_fill = t.hover;
        v.widgets.active.weak_bg_fill = t.hover;
        v.widgets.open.weak_bg_fill = t.hover;
        v.selection.bg_fill = t.checked;
        v.selection.stroke = Stroke::new(1.0, t.accent_text);
        ui.spacing_mut().button_padding = vec2(10.0, 6.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        add(ui)
    })
    .inner
}

fn label(text: &str, t: &Tokens) -> RichText {
    RichText::new(text).size(14.0).color(t.text)
}

/// A quiet button: an icon and a label.
pub fn tool_button(ui: &mut Ui, t: &Tokens, icon: Icon, text: &str) -> Response {
    ui.add(
        egui::Button::image_and_text(icon.image(ICON, t.text), label(text, t))
            .frame_when_inactive(false)
            .corner_radius(CONTROL_RADIUS)
            .min_size(vec2(0.0, BUTTON_HEIGHT)),
    )
}

/// A quiet button that stays lit while `on`, such as "Privé".
pub fn toggle_button(ui: &mut Ui, t: &Tokens, icon: Icon, text: &str, on: bool) -> Response {
    let color = if on { t.accent_text } else { t.text };
    ui.add(
        egui::Button::image_and_text(icon.image(ICON, color), RichText::new(text).size(14.0).color(color))
            .frame_when_inactive(on)
            .selected(on)
            .corner_radius(CONTROL_RADIUS)
            .min_size(vec2(0.0, BUTTON_HEIGHT)),
    )
}

/// The bar's main button, in the app colour.
pub fn primary_button(ui: &mut Ui, t: &Tokens, icon: Icon, text: &str) -> Response {
    ui.add(
        egui::Button::image_and_text(icon.image(ICON, t.on_accent), RichText::new(text).size(14.0).color(t.on_accent))
            .fill(t.accent)
            .stroke(Stroke::NONE)
            .corner_radius(CONTROL_RADIUS)
            .min_size(vec2(0.0, BUTTON_HEIGHT)),
    )
}

/// A quiet button that opens a menu, with a chevron after its label; `icon` goes before it.
pub fn menu_button<R>(ui: &mut Ui, t: &Tokens, icon: Option<Icon>, text: impl Into<WidgetText>, add: impl FnOnce(&mut Ui) -> R) -> Response {
    let text = text.into();
    let button = match icon {
        Some(icon) => egui::Button::image_and_text(icon.image(ICON, t.text), text),
        None => egui::Button::new(text),
    };
    let button = button
        .right_text(Icon::ChevronDown.image(14.0, t.text_dim))
        .frame_when_inactive(false)
        .corner_radius(CONTROL_RADIUS)
        .min_size(vec2(0.0, BUTTON_HEIGHT));
    egui::containers::menu::MenuButton::from_button(button).ui(ui, add).0
}

/// A label with a coloured dot before it, for a status or a calendar.
pub fn dotted(text: &str, dot: Color32, t: &Tokens) -> WidgetText {
    let mut job = LayoutJob::default();
    job.append("● ", 0.0, egui::TextFormat::simple(FontId::proportional(14.0), dot));
    job.append(text, 0.0, egui::TextFormat::simple(FontId::proportional(14.0), t.text));
    WidgetText::LayoutJob(job.into())
}

/// A thin upright line between groups of a command bar.
pub fn bar_separator(ui: &mut Ui, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 24.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, t.control_border));
}

/// A white card with rounded corners around `add`.
pub fn card<R>(ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin { left: 8, right: 24, top: 16, bottom: 16 })
        .show(ui, add)
}

/// A window's frame in the Helder style: the grey ground the cards sit on.
pub fn window_frame(ctx: &egui::Context, t: &Tokens) -> Frame {
    Frame::window(&ctx.global_style()).fill(t.window).stroke(Stroke::new(1.0, t.card_border)).corner_radius(CARD_RADIUS).inner_margin(12)
}

/// A form row: the icon in a column of its own, then what `add` puts after it.
pub fn icon_row<R>(ui: &mut Ui, t: &Tokens, icon: Icon, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(ROW_ICON, BUTTON_HEIGHT), Sense::hover());
        icon.image(ICON, t.text_dim).paint_at(ui, Rect::from_center_size(rect.center(), vec2(ICON, ICON)));
        add(ui)
    })
    .inner
}

/// A form row that starts with a coloured dot instead of an icon, as the calendar row does.
pub fn dot_row<R>(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(ROW_ICON, BUTTON_HEIGHT), Sense::hover());
        ui.painter().circle_filled(rect.center(), 7.0, color);
        add(ui)
    })
    .inner
}

/// Room for a row's icon, so what follows lines up under the fields.
pub fn indent(ui: &mut Ui) {
    ui.add_space(ROW_ICON + ui.spacing().item_spacing.x);
}

/// A text field with only an underline, which turns into a thicker line in the app colour
/// while it has the cursor. `large` is the size of a title.
pub fn underline_field(ui: &mut Ui, t: &Tokens, text: &mut String, hint: &str, width: f32, large: bool) -> Response {
    let size = if large { TITLE_TEXT } else { FIELD_TEXT };
    let r = ui.add(
        egui::TextEdit::singleline(text)
            .frame(Frame::NONE)
            .hint_text(RichText::new(hint).size(size).color(t.text_dim))
            .font(FontId::proportional(size))
            .text_color(t.text)
            .desired_width(width)
            .margin(Margin::symmetric(4, if large { 10 } else { 8 })),
    );
    underline(ui, t, &r);
    r
}

/// The line under a field, or under a button that opens a list.
pub fn underline(ui: &Ui, t: &Tokens, r: &Response) {
    let stroke = if r.has_focus() { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.control_border) };
    ui.painter().hline(r.rect.x_range(), r.rect.bottom(), stroke);
}

/// A choice shown as underlined text with a chevron, which opens a menu of the options.
pub fn select<R>(ui: &mut Ui, t: &Tokens, text: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> Response {
    let button = egui::Button::new(RichText::new(text).size(FIELD_TEXT).color(t.text))
        .right_text(Icon::ChevronDown.image(16.0, t.text_dim))
        .frame_when_inactive(false)
        .corner_radius(CornerRadius::ZERO)
        .min_size(vec2(width, BUTTON_HEIGHT));
    let r = egui::containers::menu::MenuButton::from_button(button).ui(ui, add).0;
    underline(ui, t, &r);
    r
}

/// A switch: a track with a knob that slides to the right and fills with the app colour when
/// on. Clicking it or its label flips `on`; a screen reader hears a checkbox.
pub fn toggle(ui: &mut Ui, t: &Tokens, on: &mut bool, text: &str) -> Response {
    let galley = WidgetText::from(RichText::new(text).size(14.0).color(t.text)).into_galley(ui, None, f32::INFINITY, TextStyle::Body);
    let track = vec2(36.0, 20.0);
    let size = vec2(track.x + 8.0 + galley.size().x, BUTTON_HEIGHT);
    let (rect, mut r) = ui.allocate_exact_size(size, Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let checked = *on;
    r.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, checked, text));
    let track = Rect::from_min_size(pos2(rect.left(), rect.center().y - track.y / 2.0), track);
    let p = ui.painter();
    let (fill, edge, knob) = if checked { (t.accent, t.accent, t.on_accent) } else { (t.card, t.control_border, t.control_border) };
    p.rect(track, CornerRadius::same(10), fill, Stroke::new(1.5, edge), egui::StrokeKind::Inside);
    let x = if checked { track.right() - 10.0 } else { track.left() + 10.0 };
    p.circle_filled(pos2(x, track.center().y), 5.5, knob);
    if r.has_focus() {
        p.rect_stroke(track.expand(3.0), CornerRadius::same(13), Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
    }
    p.galley(pos2(track.right() + 8.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
    r
}

/// A line of small, quiet text under a field.
pub fn note(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.horizontal(|ui| {
        indent(ui);
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(t.text_dim)).wrap());
    });
}

/// Lays out `add` from the right edge of what is left of the row.
pub fn right<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::right_to_left(Align::Center), add).inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_white_svg_on_a_24_grid() {
        for icon in Icon::ALL {
            let (uri, bytes) = icon.source();
            let svg = std::str::from_utf8(bytes).expect("utf-8");
            assert!(uri.ends_with(".svg") && svg.contains("viewBox=\"0 0 24 24\""), "{icon:?}");
            assert!(svg.contains("stroke=\"#ffffff\"") && !svg.contains("currentColor"), "{icon:?} is drawn white so a tint colours it");
        }
    }

    #[test]
    fn a_toggle_flips_when_clicked_and_reads_as_a_checkbox() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::new_ui_state(
            |ui, on: &mut bool| {
                toggle(ui, &Tokens::light(), on, "Hele dag");
            },
            false,
        );
        h.get_by_label("Hele dag").click();
        h.run();
        assert!(*h.state());
        h.get_by_label("Hele dag").click();
        h.run();
        assert!(!*h.state());
    }
}
