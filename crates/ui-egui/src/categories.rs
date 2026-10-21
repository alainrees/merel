//! Categories in the window: the "Categoriseren" menu in the ribbon, the coloured squares on a
//! message row, the chips above a message, and the window that manages the list.

use crate::i18n::tr;
use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Ui, vec2};
use merel_model::{Category, CategoryColor};
use serde_json::json;

use crate::{MailApp, Tokens};

/// The name of a category colour in the current language.
pub fn color_name(color: CategoryColor) -> String {
    tr(match color {
        CategoryColor::Red => "color-red",
        CategoryColor::Orange => "color-orange",
        CategoryColor::Yellow => "color-yellow",
        CategoryColor::Green => "color-green",
        CategoryColor::Blue => "color-blue",
        CategoryColor::Purple => "color-purple",
        CategoryColor::Gray => "color-gray",
    })
}

/// The colour of a category, readable on the light and the dark theme.
pub fn color_of(c: CategoryColor) -> Color32 {
    match c {
        CategoryColor::Red => Color32::from_rgb(0xd6, 0x3a, 0x3a),
        CategoryColor::Orange => Color32::from_rgb(0xe8, 0x8a, 0x2e),
        CategoryColor::Yellow => Color32::from_rgb(0xe0, 0xc0, 0x2a),
        CategoryColor::Green => Color32::from_rgb(0x3f, 0xa3, 0x5c),
        CategoryColor::Blue => Color32::from_rgb(0x3a, 0x7b, 0xd5),
        CategoryColor::Purple => Color32::from_rgb(0x8e, 0x5a, 0xc8),
        CategoryColor::Gray => Color32::from_rgb(0x8a, 0x8f, 0x98),
    }
}

/// The window that adds, recolours and removes categories.
#[derive(Clone, Debug, Default)]
pub struct Manager {
    pub name: String,
    pub color: CategoryColor,
    pub error: Option<String>,
}

impl MailApp {
    pub fn open_categories(&mut self) {
        self.categories = Some(Manager::default());
    }
}

/// Small coloured squares for a message's categories, right-aligned at `right`, on the row's
/// painter; how wide they were.
pub fn squares(app: &MailApp, p: &egui::Painter, right: egui::Pos2, names: &[String]) -> f32 {
    let mut x = right.x;
    for name in names.iter().take(6) {
        let rect = egui::Rect::from_min_size(egui::pos2(x - 10.0, right.y - 5.0), vec2(10.0, 10.0));
        p.rect_filled(rect, CornerRadius::same(2), color_of(app.engine.session.category_color(name)));
        x -= 14.0;
    }
    right.x - x
}

/// The chips above an open message, one per category; a click takes the category off.
pub fn chips(app: &mut MailApp, ui: &mut Ui, t: &Tokens, message_id: &str, names: &[String]) {
    let mut off = None;
    ui.horizontal_wrapped(|ui| {
        for name in names {
            let color = color_of(app.engine.session.category_color(name));
            let r = egui::Frame::NONE.fill(color).corner_radius(CornerRadius::same(9)).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                ui.label(RichText::new(name).size(12.0).color(Color32::WHITE));
            });
            let r = ui.interact(r.response.rect, ui.id().with(("chip", name)), Sense::click()).on_hover_text(tr("category-remove-hint"));
            if r.clicked() {
                off = Some(name.clone());
            }
        }
        ui.label(RichText::new("").color(t.text_dim));
    });
    if let Some(name) = off {
        app.run("message.categorize", json!({ "id": message_id, "name": name }));
    }
}

/// The "Categoriseren" menu in the ribbon: every known category with its colour (a mark on
/// the ones the message has), the ones the message carries that this computer does not know,
/// "Alle categorieën wissen" and "Alle categorieën…".
pub fn menu(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let selected = app.engine.session.selected().cloned();
    let has = selected.is_some();
    let carried: Vec<String> = selected.map(|m| m.categories).unwrap_or_default();
    let known: Vec<Category> = app.engine.session.categories.clone();
    let mut toggle = None;
    let mut clear = false;
    let mut manage = false;
    ui.add_enabled_ui(has, |ui| {
        crate::view::group(ui, t, &tr("categories-group"), |ui| {
            crate::helder::quiet(ui, t, |ui| {
                crate::helder::menu_button(ui, t, Some(crate::helder::Icon::Tag), tr("categorize"), |ui| {
                    ui.set_min_width(220.0);
                    for c in &known {
                        let on = carried.iter().any(|x| x.eq_ignore_ascii_case(&c.name));
                        if entry(ui, color_of(c.color), &c.name, on) {
                            toggle = Some(c.name.clone());
                        }
                    }
                    for name in carried.iter().filter(|n| !known.iter().any(|c| c.name.eq_ignore_ascii_case(n))) {
                        if entry(ui, color_of(CategoryColor::Gray), name, true) {
                            toggle = Some(name.clone());
                        }
                    }
                    ui.separator();
                    if ui.add_enabled(!carried.is_empty(), egui::Button::new(tr("categories-clear"))).clicked() {
                        clear = true;
                        ui.close();
                    }
                    if ui.button(tr("categories-all")).clicked() {
                        manage = true;
                        ui.close();
                    }
                });
            });
        });
    });
    if let Some(name) = toggle {
        app.run("message.categorize", json!({ "name": name }));
    }
    if clear {
        app.run("message.clear_categories", serde_json::Value::Null);
    }
    if manage {
        app.open_categories();
    }
}

/// One menu row: a coloured square, the name, and a mark when it is on.
fn entry(ui: &mut Ui, color: Color32, name: &str, on: bool) -> bool {
    let text = if on { format!("{name}  ✓") } else { name.to_string() };
    let r = ui.add(egui::Button::new(RichText::new(format!("    {text}"))).frame(false).min_size(vec2(200.0, 22.0)));
    let rect = egui::Rect::from_center_size(egui::pos2(r.rect.left() + 10.0, r.rect.center().y), vec2(12.0, 12.0));
    ui.painter().rect_filled(rect, CornerRadius::same(2), color);
    r.clicked()
}

/// The "All categories" window: the list with colours, a field for a new one.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut m) = app.categories.clone() else { return };
    let list: Vec<Category> = app.engine.session.categories.clone();
    let mut recolor: Option<(String, CategoryColor)> = None;
    let mut remove: Option<String> = None;
    let mut add = false;
    let mut close = false;
    if crate::helder::popup(ctx, t, "categories", &tr("view-all-categories"), 360.0, |ui| {
        if list.is_empty() {
            ui.label(RichText::new(tr("categories-none")).color(t.text_dim));
        }
        for c in &list {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(3), color_of(c.color));
                ui.add_sized([150.0, 20.0], egui::Label::new(RichText::new(&c.name).color(t.text)).truncate());
                let mut color = c.color;
                egui::ComboBox::from_id_salt(("category-color", &c.name)).selected_text(color_name(color)).width(90.0).show_ui(ui, |ui| {
                    for option in CategoryColor::ALL {
                        ui.selectable_value(&mut color, option, color_name(option));
                    }
                });
                if color != c.color {
                    recolor = Some((c.name.clone(), color));
                }
                if ui.button(tr("delete")).on_hover_text(tr("category-delete-hint")).clicked() {
                    remove = Some(c.name.clone());
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(150.0).hint_text(tr("category-new")));
            egui::ComboBox::from_id_salt("new-category-color").selected_text(color_name(m.color)).width(90.0).show_ui(ui, |ui| {
                for option in CategoryColor::ALL {
                    ui.selectable_value(&mut m.color, option, color_name(option));
                }
            });
            if ui.add(crate::helder::primary_text(t, &tr("add"))).clicked() || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                add = true;
            }
        });
        if let Some(e) = &m.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        ui.add_space(6.0);
        if ui.button(tr("close")).clicked() {
            close = true;
        }
    }) {
        close = true;
    }
    if let Some((name, color)) = recolor {
        app.run("category.save", json!({ "name": name, "color": color }));
    }
    if let Some(name) = remove {
        app.run("category.delete", json!({ "name": name }));
    }
    if add {
        match app.engine.run("category.save", &json!({ "name": m.name, "color": m.color })) {
            Ok(()) => {
                m.name.clear();
                m.error = None;
            }
            Err(_) => m.error = Some(tr("category-name-rule").to_string()),
        }
    }
    app.categories = if close { None } else { Some(m) };
    let _ = Stroke::NONE;
}
