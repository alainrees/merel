//! Urgency, as Outlook's "Hoge urgentie" and "Lage urgentie": the choice in the compose window,
//! the mark in the message list and the strip over the message.

use crate::i18n::tr;
use egui::{Color32, RichText, Ui};
use merel_model::Importance;

use crate::Tokens;

pub fn high_strip() -> String {
    tr("importance-high-strip")
}
pub fn low_strip() -> String {
    tr("importance-low-strip")
}

/// The mark the list shows before the date: a red "!" for high, a blue arrow for low.
pub fn mark(importance: Importance, t: &Tokens) -> Option<(&'static str, Color32)> {
    match importance {
        Importance::High => Some(("!", t.flag)),
        Importance::Low => Some(("↓", t.accent)),
        Importance::Normal => None,
    }
}

/// The strip over the message, as Outlook shows it; nothing for normal importance.
pub fn strip(ui: &mut Ui, t: &Tokens, importance: Importance) {
    let (text, fill) = match importance {
        Importance::High => (&high_strip(), t.flag.gamma_multiply(0.15)),
        Importance::Low => (&low_strip(), t.checked),
        Importance::Normal => return,
    };
    egui::Frame::new().fill(fill).corner_radius(crate::helder::CONTROL_RADIUS).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let color = if importance == Importance::High { t.flag } else { t.accent_text };
            ui.add(crate::helder::Icon::CircleAlert.image(16.0, color));
            ui.label(RichText::new(text).color(t.text));
        });
    });
    ui.add_space(8.0);
}

/// The small menu at the end of the compose window's subject line, as in Outlook: "Urgentie"
/// with a chevron, red "Urgentie: hoog" or blue "Urgentie: laag" once chosen. A screen reader
/// always hears the current choice.
pub fn subject_menu(ui: &mut Ui, t: &Tokens, importance: &mut Importance) {
    let (text, label, color) = match *importance {
        Importance::High => (tr("importance-high"), tr("importance-high"), t.flag),
        Importance::Low => (tr("importance-low"), tr("importance-low"), t.accent_text),
        Importance::Normal => (tr("importance"), tr("importance-normal"), t.text_dim),
    };
    let button = egui::Button::new(RichText::new(text).size(14.0).color(color))
        .right_text(crate::helder::Icon::ChevronDown.image(14.0, color))
        .frame_when_inactive(false)
        .corner_radius(crate::helder::CONTROL_RADIUS)
        .min_size(egui::vec2(0.0, 28.0));
    let (r, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        for (value, name, hint) in OPTIONS {
            if ui.selectable_label(*importance == value, tr(name)).on_hover_text(tr(hint)).clicked() {
                *importance = value;
                ui.close();
            }
        }
    });
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let _ = r.on_hover_text(tr("importance-hint"));
}

const OPTIONS: [(Importance, &str, &str); 3] = [
    (Importance::High, "importance-option-high", "importance-option-high-hint"),
    (Importance::Normal, "importance-option-normal", "importance-option-normal-hint"),
    (Importance::Low, "importance-option-low", "importance-option-low-hint"),
];
