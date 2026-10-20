//! Urgency, as Outlook's "Hoge urgentie" and "Lage urgentie": the choice in the compose window,
//! the mark in the message list and the strip over the message.

use egui::{Color32, RichText, Ui};
use mailcraft_model::Importance;

use crate::Tokens;

pub const HIGH_STRIP: &str = "Dit bericht is met hoge urgentie verzonden.";
pub const LOW_STRIP: &str = "Dit bericht is met lage urgentie verzonden.";

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
        Importance::High => (HIGH_STRIP, t.flag.gamma_multiply(0.15)),
        Importance::Low => (LOW_STRIP, t.checked),
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

/// The chooser in the compose window: a menu with Hoog, Normaal and Laag, named after the
/// current choice so an urgent message shows that it is one.
pub fn chooser(ui: &mut Ui, t: &Tokens, importance: &mut Importance) {
    let (text, color) = match *importance {
        Importance::High => ("Urgentie: hoog", t.flag),
        Importance::Low => ("Urgentie: laag", t.accent_text),
        Importance::Normal => ("Urgentie: normaal", t.text),
    };
    let label = RichText::new(text).size(14.0).color(color);
    crate::helder::menu_button(ui, t, Some(crate::helder::Icon::CircleAlert), label, |ui| {
        for (value, name, hint) in [
            (Importance::High, "Hoog", "De ontvanger ziet een rood uitroepteken bij het bericht, zoals in Outlook."),
            (Importance::Normal, "Normaal", "Geen teken bij het bericht."),
            (Importance::Low, "Laag", "De ontvanger ziet een blauw pijltje omlaag: het kan wachten."),
        ] {
            if ui.selectable_label(*importance == value, name).on_hover_text(hint).clicked() {
                *importance = value;
                ui.close();
            }
        }
    })
    .on_hover_text("Hoe dringend het bericht is; gaat mee als de kopteksten Importance en X-Priority.");
}
