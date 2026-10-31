//! The Helder style: a light grey window ground with white rounded cards, quiet buttons that
//! are a line icon and a label, one blue main button per bar, fields with only an underline,
//! and switches instead of tick boxes. The pieces here are what the windows build with, so
//! every window gets the same look; the colours come from [`Tokens`], so both themes do.
//!
//! The icons are Lucide's (ISC licence, `assets/icons/LICENSE-Lucide.txt`), drawn white so a
//! tint gives them any colour, and embedded in the binary.

use crate::i18n::tr;
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
    Archive,
    Bell,
    Calendar,
    CalendarDay,
    CalendarCheck,
    CalendarDays,
    CalendarPlus,
    CalendarRange,
    CalendarSearch,
    CalendarX,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    CircleCheck,
    Clock,
    Columns,
    Ellipsis,
    Eraser,
    Flag,
    FolderInput,
    FolderOpen,
    FolderPlus,
    Forward,
    Globe,
    Info,
    Keyboard,
    ListFilter,
    Lock,
    Mail,
    MailCheck,
    MailOpen,
    MapPin,
    Notes,
    Pencil,
    RefreshCw,
    Repeat,
    Reply,
    ReplyAll,
    Save,
    Send,
    Download,
    Search,
    Paperclip,
    FileText,
    CircleAlert,
    ShieldCheck,
    SlidersHorizontal,
    Sparkles,
    Star,
    Inbox,
    Folder,
    FilePen,
    ShieldAlert,
    SquareCheck,
    Tag,
    Trash,
    Type,
    User,
    UserPlus,
    Users,
    UsersRound,
    X,
    Zap,
    PanelRight,
    PanelBottom,
    EyeOff,
    Rows,
    MessagesSquare,
    ListTree,
    Hash,
    Monitor,
    Sun,
    Moon,
    ArrowDownUp,
    Printer,
    Phone,
    Building,
    Plus,
    ArrowLeft,
    ArrowRight,
    Undo,
    Redo,
    Paste,
    ClearFormat,
    TextColor,
    Highlighter,
    Bold,
    Italic,
    Underline,
    Strikethrough,
    BulletList,
    NumberList,
    AlignLeft,
    AlignCenter,
    AlignRight,
    IndentMore,
    IndentLess,
    AddressBook,
    TriangleAlert,
    Video,
    Link,
    Copy,
    Mic,
    MicOff,
    PhoneOff,
    Settings,
    Play,
    Pause,
}

impl Icon {
    /// Every icon, for the test that each one loads.
    pub const ALL: [Icon; 108] = [
        Icon::Archive,
        Icon::Bell,
        Icon::Calendar,
        Icon::CalendarDay,
        Icon::CalendarCheck,
        Icon::CalendarDays,
        Icon::CalendarPlus,
        Icon::CalendarRange,
        Icon::CalendarSearch,
        Icon::CalendarX,
        Icon::ChevronDown,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::CircleCheck,
        Icon::Clock,
        Icon::Columns,
        Icon::Ellipsis,
        Icon::Eraser,
        Icon::Flag,
        Icon::FolderInput,
        Icon::FolderOpen,
        Icon::FolderPlus,
        Icon::Forward,
        Icon::Globe,
        Icon::Info,
        Icon::Keyboard,
        Icon::ListFilter,
        Icon::Lock,
        Icon::Mail,
        Icon::MailCheck,
        Icon::MailOpen,
        Icon::MapPin,
        Icon::Notes,
        Icon::Pencil,
        Icon::RefreshCw,
        Icon::Repeat,
        Icon::Reply,
        Icon::ReplyAll,
        Icon::Save,
        Icon::Send,
        Icon::Download,
        Icon::Search,
        Icon::Paperclip,
        Icon::FileText,
        Icon::CircleAlert,
        Icon::ShieldCheck,
        Icon::SlidersHorizontal,
        Icon::Sparkles,
        Icon::Star,
        Icon::Inbox,
        Icon::Folder,
        Icon::FilePen,
        Icon::ShieldAlert,
        Icon::SquareCheck,
        Icon::Tag,
        Icon::Trash,
        Icon::Type,
        Icon::User,
        Icon::UserPlus,
        Icon::Users,
        Icon::UsersRound,
        Icon::X,
        Icon::Zap,
        Icon::PanelRight,
        Icon::PanelBottom,
        Icon::EyeOff,
        Icon::Rows,
        Icon::MessagesSquare,
        Icon::ListTree,
        Icon::Hash,
        Icon::Monitor,
        Icon::Sun,
        Icon::Moon,
        Icon::ArrowDownUp,
        Icon::Printer,
        Icon::Phone,
        Icon::Building,
        Icon::Plus,
        Icon::ArrowLeft,
        Icon::ArrowRight,
        Icon::Undo,
        Icon::Redo,
        Icon::Paste,
        Icon::ClearFormat,
        Icon::TextColor,
        Icon::Highlighter,
        Icon::Bold,
        Icon::Italic,
        Icon::Underline,
        Icon::Strikethrough,
        Icon::BulletList,
        Icon::NumberList,
        Icon::AlignLeft,
        Icon::AlignCenter,
        Icon::AlignRight,
        Icon::IndentMore,
        Icon::IndentLess,
        Icon::AddressBook,
        Icon::TriangleAlert,
        Icon::Video,
        Icon::Link,
        Icon::Copy,
        Icon::Mic,
        Icon::MicOff,
        Icon::PhoneOff,
        Icon::Settings,
        Icon::Play,
        Icon::Pause,
    ];

    /// The image's uri and its SVG source.
    pub fn source(self) -> (&'static str, &'static [u8]) {
        macro_rules! svg {
            ($name:literal) => {
                (concat!("bytes://icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))
            };
        }
        match self {
            Icon::Archive => svg!("archive"),
            Icon::Bell => svg!("bell"),
            Icon::Calendar => svg!("calendar"),
            Icon::CalendarDay => svg!("calendar-1"),
            Icon::CalendarCheck => svg!("calendar-check"),
            Icon::CalendarDays => svg!("calendar-days"),
            Icon::CalendarPlus => svg!("calendar-plus"),
            Icon::CalendarRange => svg!("calendar-range"),
            Icon::CalendarSearch => svg!("calendar-search"),
            Icon::CalendarX => svg!("calendar-x"),
            Icon::ChevronDown => svg!("chevron-down"),
            Icon::ChevronLeft => svg!("chevron-left"),
            Icon::ChevronRight => svg!("chevron-right"),
            Icon::CircleCheck => svg!("circle-check"),
            Icon::Clock => svg!("clock"),
            Icon::Columns => svg!("columns-3"),
            Icon::Ellipsis => svg!("ellipsis"),
            Icon::Eraser => svg!("eraser"),
            Icon::Flag => svg!("flag"),
            Icon::FolderInput => svg!("folder-input"),
            Icon::FolderOpen => svg!("folder-open"),
            Icon::FolderPlus => svg!("folder-plus"),
            Icon::Forward => svg!("forward"),
            Icon::Globe => svg!("globe"),
            Icon::Info => svg!("info"),
            Icon::Keyboard => svg!("keyboard"),
            Icon::ListFilter => svg!("list-filter"),
            Icon::Lock => svg!("lock"),
            Icon::Mail => svg!("mail"),
            Icon::MailCheck => svg!("mail-check"),
            Icon::MailOpen => svg!("mail-open"),
            Icon::MapPin => svg!("map-pin"),
            Icon::Notes => svg!("notes"),
            Icon::Pencil => svg!("pencil"),
            Icon::RefreshCw => svg!("refresh-cw"),
            Icon::Repeat => svg!("repeat"),
            Icon::Reply => svg!("reply"),
            Icon::ReplyAll => svg!("reply-all"),
            Icon::Save => svg!("save"),
            Icon::Send => svg!("send"),
            Icon::Download => svg!("download"),
            Icon::Search => svg!("search"),
            Icon::Paperclip => svg!("paperclip"),
            Icon::FileText => svg!("file-text"),
            Icon::CircleAlert => svg!("circle-alert"),
            Icon::ShieldCheck => svg!("shield-check"),
            Icon::SlidersHorizontal => svg!("sliders-horizontal"),
            Icon::Sparkles => svg!("sparkles"),
            Icon::Star => svg!("star"),
            Icon::Inbox => svg!("inbox"),
            Icon::Folder => svg!("folder"),
            Icon::FilePen => svg!("file-pen"),
            Icon::ShieldAlert => svg!("shield-alert"),
            Icon::SquareCheck => svg!("square-check"),
            Icon::Tag => svg!("tag"),
            Icon::Trash => svg!("trash"),
            Icon::Type => svg!("type"),
            Icon::User => svg!("user"),
            Icon::UserPlus => svg!("user-plus"),
            Icon::Users => svg!("users"),
            Icon::UsersRound => svg!("users-round"),
            Icon::X => svg!("x"),
            Icon::Zap => svg!("zap"),
            Icon::PanelRight => svg!("panel-right"),
            Icon::PanelBottom => svg!("panel-bottom"),
            Icon::EyeOff => svg!("eye-off"),
            Icon::Rows => svg!("rows-3"),
            Icon::MessagesSquare => svg!("messages-square"),
            Icon::ListTree => svg!("list-tree"),
            Icon::Hash => svg!("hash"),
            Icon::Monitor => svg!("monitor"),
            Icon::Sun => svg!("sun"),
            Icon::Moon => svg!("moon"),
            Icon::ArrowDownUp => svg!("arrow-down-up"),
            Icon::Printer => svg!("printer"),
            Icon::Phone => svg!("phone"),
            Icon::Building => svg!("building"),
            Icon::Plus => svg!("plus"),
            Icon::ArrowLeft => svg!("arrow-left"),
            Icon::ArrowRight => svg!("arrow-right"),
            Icon::Undo => svg!("undo-2"),
            Icon::Redo => svg!("redo-2"),
            Icon::Paste => svg!("clipboard-paste"),
            Icon::ClearFormat => svg!("remove-formatting"),
            Icon::TextColor => svg!("baseline"),
            Icon::Highlighter => svg!("highlighter"),
            Icon::Bold => svg!("bold"),
            Icon::Italic => svg!("italic"),
            Icon::Underline => svg!("underline"),
            Icon::Strikethrough => svg!("strikethrough"),
            Icon::BulletList => svg!("list"),
            Icon::NumberList => svg!("list-ordered"),
            Icon::AlignLeft => svg!("text-align-start"),
            Icon::AlignCenter => svg!("text-align-center"),
            Icon::AlignRight => svg!("text-align-end"),
            Icon::IndentMore => svg!("indent-increase"),
            Icon::IndentLess => svg!("indent-decrease"),
            Icon::AddressBook => svg!("book-user"),
            Icon::TriangleAlert => svg!("triangle-alert"),
            Icon::Video => svg!("video"),
            Icon::Link => svg!("link"),
            Icon::Copy => svg!("copy"),
            Icon::Mic => svg!("mic"),
            Icon::MicOff => svg!("mic-off"),
            Icon::Play => svg!("play"),
            Icon::Pause => svg!("pause"),
            Icon::PhoneOff => svg!("phone-off"),
            Icon::Settings => svg!("settings"),
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

/// Runs `add` with buttons drawn as the Helder style's secondary buttons, for forms and
/// settings: a white face, a thin border and rounded corners, a soft fill under the pointer.
pub fn secondary<R>(ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(CONTROL_RADIUS);
            w.expansion = 0.0;
            w.bg_stroke = Stroke::new(1.0, t.control_border);
        }
        v.widgets.inactive.weak_bg_fill = t.card;
        v.widgets.hovered.weak_bg_fill = t.hover;
        v.widgets.active.weak_bg_fill = t.checked;
        v.widgets.open.weak_bg_fill = t.hover;
        ui.spacing_mut().button_padding = vec2(12.0, 6.0);
        ui.spacing_mut().interact_size.y = 32.0;
        add(ui)
    })
    .inner
}

/// A row of a side menu: an icon and a label, lit while `on`.
pub fn nav_item(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, on: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), BUTTON_HEIGHT), Sense::click());
    r.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, on, label));
    let color = if on { t.accent_text } else { t.text };
    let fill = if on {
        Some(t.checked)
    } else if r.hovered() {
        Some(t.hover)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, CornerRadius::same(CONTROL_RADIUS), fill);
    }
    let left = rect.left() + 12.0;
    icon.image(ICON, color).paint_at(ui, Rect::from_min_size(pos2(left, rect.center().y - ICON / 2.0), vec2(ICON, ICON)));
    ui.painter().text(pos2(left + ICON + 10.0, rect.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(14.0), color);
    r
}

/// A red round mark with a "!" at `center`, `radius` across, that says something waits for
/// the person, such as a new version on the gear. A ring in the window's ground keeps it apart
/// from the icon under it.
pub fn alert_dot(ui: &Ui, t: &Tokens, center: egui::Pos2, radius: f32) {
    let painter = ui.painter();
    painter.circle(center, radius, t.flag, Stroke::new(2.0, t.window));
    painter.text(center, egui::Align2::CENTER_CENTER, "!", FontId::proportional(radius * 1.5), t.on_flag);
}

/// A count on an icon, such as the unread mail on Mail in the rail: in the app colour, in a pill
/// centred on `center`, ringed in the window colour so it stands off the icon; past 99 it
/// reads "99+".
pub fn count_badge(ui: &Ui, t: &Tokens, center: egui::Pos2, count: u32) {
    let text = if count > 99 { "99+".to_string() } else { count.to_string() };
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), t.on_accent);
    let height = 18.0;
    let width = (galley.size().x + 10.0).max(height);
    let rect = Rect::from_center_size(center + vec2((width - height) / 2.0, 0.0), vec2(width, height));
    painter.rect(rect, height / 2.0, t.accent, Stroke::new(2.0, t.window), egui::StrokeKind::Outside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, t.on_accent);
}

/// A day of the week as a round button, lit in the app colour while `on`, as Outlook's
/// repeat pane shows the days; a screen reader hears a checkbox named `text`.
pub fn day_toggle(ui: &mut Ui, t: &Tokens, text: &str, on: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    r.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, text));
    let (fill, stroke, color) = if on {
        (t.accent, Stroke::NONE, t.on_accent)
    } else {
        (if r.hovered() { t.hover } else { t.card }, Stroke::new(1.0, t.control_border), t.text)
    };
    let p = ui.painter();
    p.circle(rect.center(), 16.5, fill, stroke);
    if r.has_focus() {
        p.circle_stroke(rect.center(), 19.0, Stroke::new(2.0, t.accent));
    }
    p.text(rect.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(13.0), color);
    r
}

/// A small field for a number, in a rounded box, such as "Om de 2 weken".
pub fn number_field(ui: &mut Ui, t: &Tokens, text: &mut String, hint: &str) -> Response {
    let r = ui.add_sized(
        vec2(48.0, 30.0),
        egui::TextEdit::singleline(text)
            .frame(Frame::NONE)
            .hint_text(RichText::new(hint).size(FIELD_TEXT).color(t.text_dim))
            .font(FontId::proportional(FIELD_TEXT))
            .text_color(t.text)
            .horizontal_align(Align::Center)
            .vertical_align(Align::Center),
    );
    let stroke = if r.has_focus() { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.control_border) };
    ui.painter().rect_stroke(r.rect, CornerRadius::same(CONTROL_RADIUS), stroke, egui::StrokeKind::Inside);
    r
}

/// A link-like button that adds a row to a form, such as "E-mail toevoegen": a plus and the
/// text in the app colour.
pub fn add_link(ui: &mut Ui, t: &Tokens, text: &str) -> Response {
    ui.add(
        egui::Button::image_and_text(Icon::Plus.image(ICON, t.accent_text), RichText::new(text).size(14.0).color(t.accent_text))
            .frame_when_inactive(false)
            .corner_radius(CONTROL_RADIUS)
            .min_size(vec2(0.0, 32.0)),
    )
}

/// A thin line across a form between its sections, with some room around it.
pub fn rule(ui: &mut Ui, t: &Tokens) {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(620.0), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, t.card_border));
    ui.add_space(8.0);
}

/// The notes of a form with Outlook's formatting bar over them, as in the message and
/// appointment windows.
pub fn notes_box(ui: &mut Ui, t: &Tokens, ed: &mut crate::rich_edit::Editor, text: &mut String, width: f32, min_height: f32) -> Response {
    ui.horizontal(|ui| {
        indent(ui);
        ui.vertical(|ui| {
            ui.set_width(width);
            crate::rich_edit::bar(ui, t, ed, text);
        });
    });
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(ROW_ICON, BUTTON_HEIGHT), Sense::hover());
        Icon::Notes.image(ICON, t.text_dim).paint_at(ui, Rect::from_center_size(rect.center(), vec2(ICON, ICON)));
        ui.vertical(|ui| {
            ui.set_width(width);
            crate::rich_edit::edit(ui, t, ed, text, &tr("notes"), &tr("notes-add"), min_height)
        })
        .inner
    })
    .inner
}

/// A quiet square button with only an icon; `label` is its tooltip and its name for a screen
/// reader.
pub fn icon_button(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str) -> Response {
    let r = ui.add(
        egui::Button::image(icon.image(ICON, t.text))
            .frame_when_inactive(false)
            .corner_radius(CONTROL_RADIUS)
            .min_size(vec2(BUTTON_HEIGHT, BUTTON_HEIGHT)),
    );
    r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    r.on_hover_text(label)
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

/// A dialog's main button, only text, in the app colour: "Opslaan", "Nieuwe regel". Add it with
/// `ui.add` or `ui.add_enabled`.
pub fn primary_text(t: &Tokens, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).size(14.0).color(t.on_accent))
        .fill(t.accent)
        .stroke(Stroke::NONE)
        .corner_radius(CONTROL_RADIUS)
        .min_size(vec2(0.0, BUTTON_HEIGHT))
}

/// One of a few choices, as a row with a frame of its own: a round mark, `title` and under it
/// `detail` in quiet type. The chosen one has a frame and a filled mark in the app colour, as
/// the settings of the new Outlook have them, instead of a bare radio button.
pub fn choice(ui: &mut Ui, t: &Tokens, title: &str, detail: &str, on: bool) -> Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let width = ui.available_width();
    let frame = Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(width - 24.0);
        ui.horizontal_top(|ui| {
            let (mark, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            let p = ui.painter();
            if on {
                p.circle_filled(mark.center(), 9.0, t.accent);
                p.circle_filled(mark.center(), 3.5, t.on_accent);
            } else {
                p.circle_stroke(mark.center(), 8.0, Stroke::new(1.5, t.control_border));
            }
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // Painted, not a label: the row itself carries the name for a screen reader.
                let galley = ui.painter().layout_no_wrap(title.to_owned(), FontId::proportional(14.0), t.text);
                let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                ui.painter().galley(rect.min, galley, t.text);
                ui.add(egui::Label::new(RichText::new(detail).size(12.0).color(t.text_dim)).wrap());
            });
        });
    });
    let rect = frame.response.rect;
    let r = ui.interact(rect, ui.id().with(("choice", title)), Sense::click());
    r.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, on, title));
    let (fill, stroke) = if on {
        (t.checked, Stroke::new(2.0, t.accent))
    } else if r.hovered() {
        (t.hover, Stroke::new(1.0, t.control_border))
    } else {
        (Color32::TRANSPARENT, Stroke::new(1.0, t.card_border))
    };
    let shape = egui::epaint::RectShape::new(rect, CONTROL_RADIUS, fill, stroke, egui::StrokeKind::Inside);
    ui.painter().set(background, shape);
    if r.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), CONTROL_RADIUS, Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
    }
    r
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
    if crate::menubar::capturing() {
        crate::menubar::separator();
        return;
    }
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 24.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, t.control_border));
}

/// A white card with rounded corners around `add`.
pub fn card<R>(ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    card_frame(t).inner_margin(Margin { left: 8, right: 24, top: 16, bottom: 16 }).show(ui, add)
}

/// The frame of a card, for a panel that is one.
pub fn card_frame(t: &Tokens) -> Frame {
    Frame::new().fill(t.card).stroke(Stroke::new(1.0, t.card_border)).corner_radius(CARD_RADIUS)
}

/// Whether a ribbon button shows only its icon; its name is then a tooltip and stays its label
/// for a screen reader. These are the actions everyone knows by their icon.
pub fn ribbon_icon_only(label: &str) -> bool {
    let label = &*crate::i18n::dutch(label);
    matches!(label, "Verwijderen" | "Archiveren" | "Ongewenst" | "Gelezen" | "Ongelezen" | "Vlag" | "Beheren…" | "Systeem" | "Licht" | "Donker")
}

/// Whether a ribbon button is its ribbon's blue main button: making something new.
pub fn ribbon_primary(label: &str) -> bool {
    let label = &*crate::i18n::dutch(label);
    matches!(label, "Nieuwe e-mail" | "Nieuwe afspraak" | "Nieuw contact" | "Nieuwe taak" | "Nieuwe chat" | "Vergadering plannen")
}

/// The icon of a ribbon button, by its label, else by the glyph it had before the Helder style.
pub fn ribbon_icon(label: &str, glyph: &str) -> Option<Icon> {
    let label = &*crate::i18n::dutch(label);
    Some(match label {
        "Nieuwe e-mail" | "E-mail sturen" => Icon::Mail,
        "Beantwoorden" => Icon::Reply,
        "Allen beantwoorden" => Icon::ReplyAll,
        "Doorsturen" => Icon::Forward,
        "Archiveren" => Icon::Archive,
        "Ongewenst" => Icon::ShieldAlert,
        "Verplaatsen" => Icon::FolderInput,
        "Regels" => Icon::ListFilter,
        "Gelezen" => Icon::MailOpen,
        "Ongelezen" => Icon::Mail,
        "Alles gelezen" => Icon::MailCheck,
        "Vlag" => Icon::Flag,
        "Nieuwe afspraak" => Icon::CalendarPlus,
        "Vandaag" => Icon::CalendarCheck,
        "Komende 7 dagen" => Icon::CalendarRange,
        "Vergadering plannen" => Icon::Video,
        "Deelnemen via link" => Icon::Link,
        "Ga naar datum" => Icon::CalendarSearch,
        "Dag" => Icon::CalendarDay,
        "Werkweek" => Icon::Columns,
        "Week" => Icon::CalendarRange,
        "Maand" => Icon::CalendarDays,
        "Vernieuwen" | "Alles synchroniseren" => Icon::RefreshCw,
        "Sneltoetsen" => Icon::Keyboard,
        "Over Merel" => Icon::Info,
        "Op GitHub" => Icon::Globe,
        "Nieuw contact" => Icon::UserPlus,
        "Nieuwe groep" => Icon::UsersRound,
        "Nieuwe taak" => Icon::CircleCheck,
        "Nieuwe map" => Icon::FolderPlus,
        "Map leegmaken" => Icon::Eraser,
        "Rechts" => Icon::PanelRight,
        "Onder" => Icon::PanelBottom,
        "Uit" => Icon::EyeOff,
        "Compact" => Icon::Rows,
        "Gesprekken" | "Nieuwe chat" | "Chatten" => Icon::MessagesSquare,
        "Alle chats" => Icon::ArrowLeft,
        "Groepen" => Icon::ListTree,
        "Weeknummers" => Icon::Hash,
        "Werkuren" => Icon::Clock,
        "Systeem" | "Systeemtaal" => Icon::Monitor,
        "Licht" => Icon::Sun,
        "Donker" => Icon::Moon,
        "Opruimen" => Icon::Sparkles,
        _ => match glyph {
            "🗑" => Icon::Trash,
            "✎" => Icon::Pencil,
            "📂" => Icon::FolderOpen,
            "☑" => Icon::SquareCheck,
            "⚡" => Icon::Zap,
            // A folder that is not a favourite yet; a favourite keeps its filled ★.
            "☆" => Icon::Star,
            "⚙" => Icon::Ellipsis,
            _ => return None,
        },
    })
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

/// A rounded search field with a magnifier, as the Helder title bar has it.
pub fn search_field(ui: &mut Ui, t: &Tokens, text: &mut String, id: egui::Id, hint: &str, width: f32) -> Response {
    Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.control_border))
        .corner_radius(18)
        .inner_margin(Margin { left: 12, right: 12, top: 2, bottom: 2 })
        .show(ui, |ui| {
            ui.set_width(width - 26.0);
            ui.horizontal(|ui| {
                ui.add(Icon::Search.image(16.0, t.text_dim));
                ui.add(
                    egui::TextEdit::singleline(text)
                        .id(id)
                        .frame(Frame::NONE)
                        .hint_text(RichText::new(hint).color(t.text_dim))
                        .text_color(t.text)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(2, 6)),
                )
            })
            .inner
        })
        .inner
}

/// The colours of the round avatars before a sender's name; each carries white initials at
/// WCAG AA.
pub const AVATAR_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x1F, 0x5F, 0xA8),
    Color32::from_rgb(0x2E, 0x6E, 0x50),
    Color32::from_rgb(0x6B, 0x4F, 0xA8),
    Color32::from_rgb(0x8A, 0x53, 0x00),
    Color32::from_rgb(0xA1, 0x1F, 0x35),
    Color32::from_rgb(0x0F, 0x66, 0x6E),
    Color32::from_rgb(0x4A, 0x4E, 0x54),
    Color32::from_rgb(0x8C, 0x2F, 0x86),
];

/// One or two letters for a person: the first letters of the first and last word of the
/// name, or the first letter of the address.
pub fn initials(name: &str) -> String {
    // What stands in brackets, such as the company in "Alain Rees (Osso & Picca)", is not the name.
    let name = name.split('(').next().unwrap_or(name);
    let words: Vec<&str> = name.split_whitespace().filter(|w| w.chars().next().is_some_and(char::is_alphanumeric)).collect();
    let first = |w: &str| w.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    match words.as_slice() {
        [] => String::new(),
        [one] => first(one),
        [a, .., b] => first(a) + &first(b),
    }
}

/// A round avatar with `name`'s initials, its colour fixed by `email` so a person keeps theirs.
pub fn avatar(p: &egui::Painter, center: egui::Pos2, radius: f32, name: &str, email: &str) {
    let hash = email.to_lowercase().bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    let color = AVATAR_COLORS[hash as usize % AVATAR_COLORS.len()];
    p.circle_filled(center, radius, color);
    let text = if initials(name).is_empty() { initials(email) } else { initials(name) };
    p.text(center, egui::Align2::CENTER_CENTER, text, FontId::proportional(radius * 0.8), Color32::WHITE);
}

/// A person's picture: their photo, cut round, else the avatar with their initials. The
/// photo's bytes are handed over once per photo, kept in the context under its fingerprint.
pub fn portrait(ui: &Ui, center: egui::Pos2, radius: f32, name: &str, email: &str, photo: Option<&merel_model::Photo>) {
    let Some(photo) = photo else {
        avatar(ui.painter(), center, radius, name, email);
        return;
    };
    let key = egui::Id::new(("portrait", photo.fingerprint()));
    let bytes: std::sync::Arc<[u8]> = match ui.data(|d| d.get_temp::<std::sync::Arc<[u8]>>(key)) {
        Some(b) => b,
        None => {
            let b: std::sync::Arc<[u8]> = photo.0.clone().into();
            ui.data_mut(|d| d.insert_temp(key, b.clone()));
            b
        }
    };
    let ext = photo.mime().and_then(|m| m.strip_prefix("image/")).unwrap_or("jpeg");
    let uri = format!("bytes://merel/portrait-{:016x}.{ext}", photo.fingerprint());
    let rect = Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
    // Until it is decoded, and if it cannot be, the initials show underneath.
    avatar(ui.painter(), center, radius, name, email);
    Image::from_bytes(uri, egui::load::Bytes::Shared(bytes))
        .fit_to_exact_size(rect.size())
        .maintain_aspect_ratio(false)
        .corner_radius(CornerRadius::same(radius.min(127.0) as u8))
        .paint_at(ui, rect);
}

/// A tab of a card, such as Contactpersoon and Organisatie: its name, underlined in the app
/// colour while it is the one shown.
pub fn tab(ui: &mut Ui, t: &Tokens, text: &str, on: bool) -> Response {
    let color = if on { t.accent_text } else { t.text_dim };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(15.0), color);
    let (rect, r) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 36.0), Sense::click());
    r.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, on, text));
    let p = ui.painter();
    p.galley(egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0), galley, color);
    if on {
        p.hline(rect.x_range(), rect.bottom() - 1.5, Stroke::new(3.0, t.accent));
    } else if r.hovered() {
        p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, t.control_border));
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of choices shown as rounded chips, such as Alles, Ongelezen; the lit one is `on`.
pub fn chip(ui: &mut Ui, t: &Tokens, text: &str, on: bool) -> Response {
    let color = if on { t.accent_text } else { t.text };
    ui.add(
        egui::Button::new(RichText::new(text).size(13.0).color(color))
            .fill(if on { t.checked } else { Color32::TRANSPARENT })
            .frame_when_inactive(on)
            .selected(on)
            .stroke(Stroke::NONE)
            .corner_radius(14)
            .min_size(vec2(0.0, 28.0)),
    )
}

/// A file as a tile: its kind, name and size. Its label is `label`, for screen readers and
/// the tests.
pub fn file_tile(ui: &mut Ui, t: &Tokens, name: &str, size: &str, label: &str, enabled: bool) -> Response {
    let font = FontId::proportional(13.0);
    let small = FontId::proportional(12.0);
    let name_w = ui.painter().layout_no_wrap(name.to_owned(), font.clone(), Color32::WHITE).size().x;
    let width = (10.0 + 28.0 + 10.0 + name_w.min(220.0) + 14.0).max(140.0);
    let (rect, r) = ui.allocate_exact_size(vec2(width, 44.0), if enabled { Sense::click() } else { Sense::hover() });
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let p = ui.painter();
    let fill = if r.hovered() && enabled { t.hover } else { t.card };
    p.rect(rect, CornerRadius::same(CONTROL_RADIUS), fill, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    let badge = egui::Rect::from_min_size(egui::pos2(rect.left() + 10.0, rect.center().y - 14.0), vec2(28.0, 28.0));
    p.rect_filled(badge, 6, t.checked);
    let kind: String = std::path::Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase().chars().take(4).collect();
    p.text(badge.center(), egui::Align2::CENTER_CENTER, kind, FontId::proportional(9.0), t.accent_text);
    let x = badge.right() + 10.0;
    let text_color = if enabled { t.text } else { t.text_disabled };
    let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(rect.right() - 8.0, rect.bottom()));
    p.with_clip_rect(clip).text(egui::pos2(x, rect.top() + 7.0), egui::Align2::LEFT_TOP, name, font, text_color);
    p.text(egui::pos2(x, rect.top() + 25.0), egui::Align2::LEFT_TOP, size, small, t.text_dim);
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

/// A window of its own, as Outlook opens a message, an appointment or a new mail: the system's
/// title bar with its close, minimise and zoom buttons and `title`, on macOS, Windows and Linux
/// alike, and no title bar of ours under it. Closing it from the title bar clears `open`.
/// Where the platform cannot open a second window (and in the tests) it is drawn as a window
/// inside the main one. A closed window is hidden first and let go a few frames later (see
/// [`linger_closed_windows`]).
pub fn item_window<R>(
    ctx: &egui::Context,
    t: &Tokens,
    id: &str,
    title: &str,
    size: egui::Vec2,
    open: &mut bool,
    add: impl FnMut(&mut Ui) -> R,
) -> Option<R> {
    own_window(ctx, t, id, title, Shape::Item(size), open, add)
}

/// A dialog in a window of its own, such as Regels or Snelle stappen: the system's title bar with
/// its close, minimise and zoom buttons and `title`, and under it the Helder style, a card on
/// the window's grey ground whose buttons are secondary buttons. The window is `width` wide and
/// as tall as what `add` draws, and opens over the middle of the main window. Returns whether it
/// was closed from its title bar. Where the platform cannot open a second window (and in the
/// tests) it is drawn as a window inside the main one.
pub fn popup(ctx: &egui::Context, t: &Tokens, id: &str, title: &str, width: f32, add: impl FnMut(&mut Ui)) -> bool {
    let mut open = true;
    own_window(ctx, t, id, title, Shape::Popup(width, true), &mut open, add);
    !open
}

/// What kind of window [`own_window`] opens.
#[derive(Clone, Copy)]
enum Shape {
    /// An item, such as a message or an appointment: this size to start with, then the user's.
    Item(egui::Vec2),
    /// A dialog this wide and as tall as its content; on a card, or on the grey ground for a
    /// dialog that lays out cards of its own.
    Popup(f32, bool),
}

/// The room around a popup's card, and the card's own margin.
const POPUP_GROUND: f32 = 12.0;
const POPUP_CARD: f32 = 16.0;

fn own_window<R>(
    ctx: &egui::Context,
    t: &Tokens,
    id: &str,
    title: &str,
    shape: Shape,
    open: &mut bool,
    mut add: impl FnMut(&mut Ui) -> R,
) -> Option<R> {
    // An item opens over the main window, a little in from its corner, as Outlook opens one; a
    // dialog over its middle. The system could otherwise put the window where it is not seen,
    // off screen or behind the main one. Inside the main window (embedded) egui places it.
    let main = if ctx.embed_viewports() { None } else { ctx.input(|i| i.viewport().outer_rect) };
    let height_id = egui::Id::new(id).with("popup-height");
    let height: Option<f32> = ctx.data(|d| d.get_temp(height_id));
    let size = match shape {
        Shape::Item(size) => size,
        Shape::Popup(width, _) => vec2(width + 2.0 * (POPUP_GROUND + POPUP_CARD), height.unwrap_or(200.0)),
    };
    // A dialog is placed for good once its height is known, so it is centred as it is.
    let settled = matches!(shape, Shape::Item(_)) || height.is_some();
    let (viewport, place, new) = ctx.data_mut(|d| {
        let w = d.get_temp_mut_or_default::<Windows>(egui::Id::new(WINDOWS));
        // Each opening is a window of its own. Reusing the one just closed, still kept hidden
        // for a few frames, left it hidden on macOS: the item opened in a window nobody saw.
        let base = egui::Id::new(id);
        let open = w.open.iter().find(|(b, v)| *b == base && w.last.contains(v)).map(|(_, v)| *v);
        let new = open.is_none();
        let viewport = open.unwrap_or_else(|| {
            w.next += 1;
            let v = egui::ViewportId::from_hash_of((id, w.next));
            w.open.retain(|(b, _)| *b != base);
            w.open.push((base, v));
            v
        });
        w.shown.push(viewport);
        let place = match w.placed.iter().find(|(v, _)| *v == viewport) {
            Some((_, at)) => Some(*at),
            None => {
                let at = main.map(|r| match shape {
                    Shape::Item(_) => r.min + vec2(60.0, 60.0),
                    Shape::Popup(..) => (r.center() - size / 2.0).max(r.min + vec2(20.0, 60.0)),
                });
                if let Some(at) = at
                    && settled
                {
                    w.placed.push((viewport, at));
                }
                at
            }
        };
        (viewport, place, new)
    });
    let mut builder = egui::ViewportBuilder::default().with_title(title).with_visible(true).with_inner_size(size);
    // A dialog keeps all three title bar buttons and can be made larger; it takes its content's
    // height again when that changes.
    if let Shape::Item(_) = shape {
        builder = builder.with_min_inner_size(vec2(480.0, 360.0));
    }
    if let Some(at) = place {
        builder = builder.with_position(at);
    }
    if new && !ctx.embed_viewports() {
        // A window that just opened comes to the front, above the main window that was clicked.
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
    }
    // egui can run a frame of the window twice (a discarded first pass, as when a widget's size
    // settles), so `add` draws on every pass: a pass that drew nothing showed the window black.
    let mut out = None;
    ctx.show_viewport_immediate(viewport, builder, |ui, class| {
        if class == egui::ViewportClass::EmbeddedWindow {
            out = Some(match shape {
                Shape::Item(_) => add(ui),
                Shape::Popup(width, _) => {
                    ui.set_width(width);
                    secondary(ui, t, &mut add)
                }
            });
            return;
        }
        if ui.input(|i| i.viewport().close_requested()) {
            *open = false;
        }
        match shape {
            Shape::Item(_) => {
                egui::CentralPanel::default().frame(Frame::NONE.fill(t.window).inner_margin(12)).show(ui, |ui| {
                    out = Some(add(ui));
                });
            }
            Shape::Popup(width, card) => {
                egui::CentralPanel::default().frame(Frame::NONE.fill(t.window).inner_margin(POPUP_GROUND)).show(ui, |ui| {
                    let frame = if card { card_frame(t) } else { Frame::NONE };
                    let card = frame.inner_margin(POPUP_CARD).show(ui, |ui| {
                        ui.set_width(width);
                        secondary(ui, t, &mut add)
                    });
                    out = Some(card.inner);
                    // The window takes the height of what it shows.
                    let height = (card.response.rect.height() + 2.0 * POPUP_GROUND).ceil();
                    if (height - size.y).abs() > 0.5 {
                        ui.ctx().data_mut(|d| d.insert_temp(height_id, height));
                        ui.ctx().request_repaint();
                    }
                });
            }
        }
    });
    out
}

const WINDOWS: &str = "helder-item-windows";
/// How many frames a closed window stays, hidden, before it is let go.
const LINGER_FRAMES: u32 = 10;

/// The item windows shown this frame and the last, and the closed ones still kept.
#[derive(Clone, Default)]
struct Windows {
    shown: Vec<egui::ViewportId>,
    last: Vec<egui::ViewportId>,
    lingering: Vec<(egui::ViewportId, u32)>,
    /// Where each open window was put when it opened, so it stays where the user moves it.
    placed: Vec<(egui::ViewportId, egui::Pos2)>,
    /// The window each item window's id has now, and the count that makes the next one new.
    open: Vec<(egui::Id, egui::ViewportId)>,
    next: u64,
}

/// Keeps each item window that just closed for a few more frames, hidden, before it is let go.
/// Closing a window at once, while AppKit still watches it for the Touch Bar, can abort the
/// app on macOS; ordering it out first and letting go of it later, as winit and Zed's fixes
/// for the same crash do, avoids that. Call once per frame, after every window was shown.
pub fn linger_closed_windows(ctx: &egui::Context) {
    let id = egui::Id::new(WINDOWS);
    let mut w: Windows = ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<Windows>(id)));
    if !ctx.embed_viewports() {
        for closed in w.last.iter().filter(|v| !w.shown.contains(v)) {
            w.lingering.push((*closed, LINGER_FRAMES));
        }
        w.lingering.retain(|(v, _)| !w.shown.contains(v));
        for (viewport, frames) in &mut w.lingering {
            *frames -= 1;
            ctx.show_viewport_immediate(*viewport, egui::ViewportBuilder::default().with_visible(false), |_, _| {});
            ctx.request_repaint();
        }
        w.lingering.retain(|(_, frames)| *frames > 0);
    }
    w.placed.retain(|(v, _)| w.shown.contains(v));
    w.open.retain(|(_, v)| w.shown.contains(v));
    w.last = std::mem::take(&mut w.shown);
    ctx.data_mut(|d| d.insert_temp(id, w));
}

/// Lays out `add` at the right end of a row of buttons. Unlike [`right`] it never draws over
/// what came before: it takes the width it needed last frame, and when the row has no room
/// left it follows straight on, so the window grows rather than the buttons overlapping.
pub fn trailing<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let id = ui.id().with("trailing");
    let gap = ui.spacing().item_spacing.x;
    // Until the group has been measured once, it follows on, so it cannot stretch the window.
    let width: Option<f32> = ui.ctx().data(|d| d.get_temp(id));
    ui.add_space(width.map_or(gap, |w| (ui.available_width() - w - gap).max(gap)));
    let width = width.unwrap_or(-1.0);
    let inner = ui.scope(add);
    let used = inner.response.rect.width();
    if (used - width).abs() > 0.5 {
        ui.ctx().data_mut(|d| d.insert_temp(id, used));
        ui.ctx().request_discard("the trailing buttons changed width");
    }
    inner.inner
}

/// Lays out `add` from the right edge of what is left of the row.
pub fn right<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::right_to_left(Align::Center), add).inner
}

/// The accessible name of a dialog's close button.
pub fn close_label() -> String {
    tr("close")
}

/// A dialog in the Helder style in a window of its own (see [`popup`]), such as "Over Merel" or
/// the welcome. Inside the main window (embedded) it keeps a quiet close button of its own, as
/// such a window has no title bar buttons. Returns whether it was closed.
pub fn dialog(ctx: &egui::Context, t: &Tokens, title: &str, width: f32, mut add: impl FnMut(&mut Ui)) -> bool {
    let mut close = false;
    let mut open = true;
    own_window(ctx, t, title, title, Shape::Popup(width, false), &mut open, |ui| {
        // Drawn inside the main window, it is still on the main window's viewport.
        if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(TITLE_TEXT).strong().color(t.text));
                right(ui, |ui| {
                    close = icon_button(ui, t, Icon::X, &close_label()).clicked();
                });
            });
            ui.add_space(8.0);
        }
        add(ui);
    });
    close || !open
}

/// A round badge in the selection colour with `icon` in the app colour, for the head of a
/// dialog's card.
pub fn badge(ui: &mut Ui, t: &Tokens, icon: Icon) {
    let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 24.0, t.checked);
    icon.image(24.0, t.accent_text).paint_at(ui, Rect::from_center_size(rect.center(), vec2(24.0, 24.0)));
}

/// The app icon, the folded blackbird, at the size of a [`badge`]: for "Over Merel" and the
/// first step of the welcome.
pub fn app_badge(ui: &mut Ui) {
    const ICON: &[u8] = include_bytes!("../../../packaging/icon-256.png");
    let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
    Image::from_bytes("bytes://merel/app-icon.png", ICON).fit_to_exact_size(vec2(56.0, 56.0)).paint_at(ui, rect.expand(4.0));
}

/// A row of dots for the steps of a wizard: the steps done and the current one in the app
/// colour, the current one wider. A screen reader hears "Stap n van m".
pub fn step_dots(ui: &mut Ui, t: &Tokens, current: usize, count: usize) {
    let text = crate::i18n::tr_args("step-of", &[("step", (current + 1).into()), ("count", count.into())]);
    let (rect, r) = ui.allocate_exact_size(vec2(count as f32 * 14.0 + 10.0, 12.0), Sense::hover());
    r.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
    let mut x = rect.left();
    for i in 0..count {
        let w = if i == current { 18.0 } else { 8.0 };
        let fill = if i <= current { t.accent } else { t.control_border };
        ui.painter().rect_filled(Rect::from_min_size(pos2(x, rect.center().y - 4.0), vec2(w, 8.0)), CornerRadius::same(4), fill);
        x += w + 6.0;
    }
}

/// A progress bar in the app colour on a quiet track; `None` when the total is unknown shows a
/// band that moves. A screen reader hears the percentage.
pub fn progress_bar(ui: &mut Ui, t: &Tokens, fraction: Option<f32>) {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
    let text = fraction.map_or_else(|| tr("busy"), |f| format!("{:.0}%", f.clamp(0.0, 1.0) * 100.0));
    r.widget_info(|| WidgetInfo::labeled(WidgetType::ProgressIndicator, true, &text));
    ui.painter().rect_filled(rect, CornerRadius::same(4), t.control_border);
    let fill = match fraction {
        Some(f) => Rect::from_min_size(rect.min, vec2(rect.width() * f.clamp(0.0, 1.0), rect.height())),
        None => {
            let phase = (ui.input(|i| i.time) as f32 * 0.6).fract();
            ui.ctx().request_repaint();
            let w = rect.width() * 0.3;
            Rect::from_min_size(pos2(rect.left() + (rect.width() - w) * phase, rect.top()), vec2(w, rect.height()))
        }
    };
    ui.painter().rect_filled(fill, CornerRadius::same(4), t.accent);
}

/// A quiet button with only text, no frame until the pointer is on it: the lesser choices of a
/// dialog, such as "Overslaan".
pub fn text_button(ui: &mut Ui, t: &Tokens, text: &str) -> Response {
    ui.add(egui::Button::new(label(text, t)).frame_when_inactive(false).corner_radius(CONTROL_RADIUS).min_size(vec2(0.0, BUTTON_HEIGHT)))
}

/// Text in a dialog's card: body size, wrapped to the card.
pub fn paragraph(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(14.0).color(t.text)).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_carry_white_initials_at_wcag_aa() {
        for c in AVATAR_COLORS {
            assert!(crate::theme::contrast(Color32::WHITE, c) >= 4.5, "{c:?}");
        }
        assert_eq!(initials("Marieke de Vries"), "MV");
        assert_eq!(initials("Studio"), "S");
        assert_eq!(initials("Sanne de Vries (Studio ArtCraft)"), "SV");
        assert_eq!(initials("  "), "");
    }

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
