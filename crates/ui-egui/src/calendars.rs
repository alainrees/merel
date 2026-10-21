//! The calendar list in the calendar's side pane, as Outlook has it: every calendar a row with
//! a round dot in its own colour, filled with a tick while its appointments show and a ring
//! while they are hidden. The "⋯" after a row (or a right click on it) picks another colour, and
//! for a calendar on this computer renames or deletes it. "Agenda toevoegen" makes a new one.

use egui::{Color32, FontId, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use merel_engine::CommandError;
use merel_model::{CALENDAR_COLORS, Calendar, MAX_CALENDAR_NAME};

use crate::helder::{self, Icon};
use crate::{MailApp, Tokens};

/// The button above the list, and the title of its window.
pub const ADD: &str = "Agenda toevoegen";
pub const RENAME: &str = "Naam van agenda wijzigen";
pub const DELETE: &str = "Agenda verwijderen";
/// The button that confirms deleting.
pub const CONFIRM_DELETE: &str = "Ja, verwijderen";

/// The window that is open over the list, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    Add { name: String, color: [u8; 3] },
    Rename { id: String, name: String },
    Delete { id: String, name: String },
}

/// The label of a row's options button.
pub fn options_label(name: &str) -> String {
    format!("Opties voor {name}")
}

/// The label of a colour in the colour menu.
pub fn color_label(name: &str) -> String {
    format!("Kleur {name}")
}

/// The colour a picked RGB value shows in.
pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The size of a row's options button.
const MORE: f32 = 28.0;

/// The tick on a filled dot: white, which stands out at least 3:1 on every colour offered.
const TICK: Color32 = Color32::WHITE;

/// The round dot of a calendar: filled with a tick when shown, a ring when hidden.
pub fn paint_dot(p: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32, shown: bool) {
    if shown {
        p.circle_filled(center, radius, color);
        let s = radius * 0.5;
        let points = vec![pos2(center.x - s, center.y), pos2(center.x - s * 0.25, center.y + s * 0.7), pos2(center.x + s, center.y - s * 0.6)];
        p.add(egui::Shape::line(points, Stroke::new(1.6, TICK)));
    } else {
        p.circle_stroke(center, radius - 1.0, Stroke::new(2.0, color));
    }
}

/// "Agenda toevoegen" and the list of calendars, each with its dot and options.
pub fn list(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    helder::quiet(ui, t, |ui| {
        if helder::tool_button(ui, t, Icon::CalendarPlus, ADD).clicked() {
            let used: Vec<[u8; 3]> = app.engine.session.calendars.iter().map(|c| c.color).collect();
            let color = CALENDAR_COLORS.iter().map(|(_, c)| *c).find(|c| !used.contains(c)).unwrap_or(CALENDAR_COLORS[0].1);
            app.calendar.dialog = Some(Dialog::Add { name: String::new(), color });
        }
    });
    let calendars: Vec<Calendar> = app.engine.session.calendars.clone();
    let locals = calendars.iter().filter(|c| c.account_id.is_none()).count();
    // As Outlook: the calendars of each account under its address, the ones that live only
    // here under a heading of their own, so two accounts with the same name stay apart.
    let mut groups: Vec<(String, Vec<&Calendar>)> = vec![(LOCAL.into(), calendars.iter().filter(|c| c.account_id.is_none()).collect())];
    for a in &app.engine.session.accounts {
        groups.push((a.address.email().to_string(), calendars.iter().filter(|c| c.account_id.as_deref() == Some(a.id.as_str())).collect()));
    }
    for (heading, members) in groups.into_iter().filter(|(_, m)| !m.is_empty()) {
        ui.add_space(6.0);
        let r = ui.add(egui::Label::new(RichText::new(&heading).size(12.0).strong().color(t.text_dim)).truncate());
        if heading == LOCAL {
            r.on_hover_text(LOCAL_HINT);
        }
        ui.add_space(2.0);
        for c in members {
            row(app, ui, t, c, c.account_id.is_none() && locals > 1);
        }
    }
}

/// The heading over the calendars that live only on this computer.
pub const LOCAL: &str = "Op deze computer";
/// What that heading explains on hover.
pub const LOCAL_HINT: &str = "Deze agenda's staan alleen op deze computer en worden niet met een account gedeeld.";

/// One calendar: a click on the dot or the name shows or hides its appointments.
fn row(app: &mut MailApp, ui: &mut Ui, t: &Tokens, c: &Calendar, deletable: bool) {
    let shown = app.view.calendar_shown(&c.id);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // Room for the options button after the row, so the pane never grows to fit it.
        let width = (ui.available_width() - MORE - 2.0).max(60.0);
        let (rect, r) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, shown, &c.name));
        let p = ui.painter();
        if r.hovered() {
            p.rect_filled(rect, helder::CONTROL_RADIUS, t.hover);
        }
        if r.has_focus() {
            p.rect_stroke(rect, helder::CONTROL_RADIUS, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
        paint_dot(p, pos2(rect.left() + 14.0, rect.center().y), 8.0, rgb(c.color), shown);
        let text = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(rect.right() - 4.0, rect.bottom()));
        let galley = p.layout_no_wrap(c.name.clone(), FontId::proportional(14.0), t.text);
        p.with_clip_rect(text).galley(pos2(text.left(), rect.center().y - galley.size().y / 2.0), galley, t.text);
        let r = r.on_hover_text(crate::calendar::CALENDAR_TICK_HINT);
        if r.clicked() {
            app.view.set_calendar_shown(&c.id, !shown);
            let ctx = ui.ctx().clone();
            app.save_view(&ctx);
        }
        r.context_menu(|ui| menu(app, ui, t, c, deletable));
        let button = egui::Button::image(Icon::Ellipsis.image(16.0, t.text_dim)).frame_when_inactive(false).min_size(vec2(MORE, MORE));
        let (more, _) = helder::quiet(ui, t, |ui| {
            ui.spacing_mut().button_padding = vec2((MORE - 16.0) / 2.0, (MORE - 16.0) / 2.0);
            egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| menu(app, ui, t, c, deletable))
        });
        let label = options_label(&c.name);
        more.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    });
}

/// The colours to pick from, then rename and delete for a calendar on this computer.
fn menu(app: &mut MailApp, ui: &mut Ui, t: &Tokens, c: &Calendar, deletable: bool) {
    ui.label(RichText::new("Kleur").size(12.0).strong().color(t.text_dim));
    egui::Grid::new(("calendar-colors", &c.id)).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        for (i, (name, color)) in CALENDAR_COLORS.iter().enumerate() {
            let (rect, r) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
            let label = color_label(name);
            let on = c.color == *color;
            r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, &label));
            if r.hovered() || r.has_focus() {
                ui.painter().circle_stroke(rect.center(), 12.0, Stroke::new(2.0, t.text));
            }
            paint_dot(ui.painter(), rect.center(), 9.0, rgb(*color), on);
            if r.on_hover_text(*name).clicked() {
                app.run("calendar.color", json!({ "id": c.id, "color": color }));
                ui.close();
            }
            if i % 6 == 5 {
                ui.end_row();
            }
        }
    });
    if c.account_id.is_some() {
        return;
    }
    ui.separator();
    if ui.add(egui::Button::image_and_text(Icon::Pencil.image(helder::ICON, t.text), "Naam wijzigen…")).clicked() {
        app.calendar.dialog = Some(Dialog::Rename { id: c.id.clone(), name: c.name.clone() });
        ui.close();
    }
    if deletable && ui.add(egui::Button::image_and_text(Icon::Trash.image(helder::ICON, t.text), "Verwijderen…")).clicked() {
        app.calendar.dialog = Some(Dialog::Delete { id: c.id.clone(), name: c.name.clone() });
        ui.close();
    }
}

fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("name") => "Geef de agenda een naam.".into(),
        CommandError::Param("id") => "Deze agenda kan dat niet; er blijft altijd één agenda op deze computer.".into(),
        CommandError::NotFound => "Deze agenda bestaat niet meer.".into(),
        e => e.to_string(),
    }
}

/// The window to add, rename or delete a calendar, while one is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut dialog) = app.calendar.dialog.take() else { return };
    let title = match &dialog {
        Dialog::Add { .. } => ADD,
        Dialog::Rename { .. } => RENAME,
        Dialog::Delete { .. } => DELETE,
    };
    let mut ok = false;
    let mut close = false;
    egui::Window::new(title).collapsible(false).resizable(false).default_width(320.0).frame(helder::window_frame(ctx, t)).show(ctx, |ui| {
        match &mut dialog {
            Dialog::Add { name, color } => {
                let r = helder::underline_field(ui, t, name, "Naam van de agenda", 280.0, false);
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    ok = true;
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for (label, c) in CALENDAR_COLORS {
                        let (rect, r) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                        let text = color_label(label);
                        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, *color == c, &text));
                        if r.hovered() || r.has_focus() {
                            ui.painter().circle_stroke(rect.center(), 12.0, Stroke::new(2.0, t.text));
                        }
                        paint_dot(ui.painter(), rect.center(), 9.0, rgb(c), *color == c);
                        if r.on_hover_text(label).clicked() {
                            *color = c;
                        }
                    }
                });
            }
            Dialog::Rename { name, .. } => {
                let r = helder::underline_field(ui, t, name, "Naam van de agenda", 280.0, false);
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    ok = true;
                }
            }
            Dialog::Delete { id, name } => {
                let count = app.engine.session.appointments.iter().filter(|a| &a.calendar_id == id).count();
                let text = match count {
                    0 => format!("Agenda \"{name}\" verwijderen?"),
                    1 => format!("Agenda \"{name}\" en de afspraak erin verwijderen? Dit kan niet ongedaan worden gemaakt."),
                    n => format!("Agenda \"{name}\" en de {n} afspraken erin verwijderen? Dit kan niet ongedaan worden gemaakt."),
                };
                ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
            }
        }
        if name_too_long(&dialog) {
            ui.label(RichText::new(format!("Een naam is hooguit {MAX_CALENDAR_NAME} tekens.")).size(12.0).color(t.flag));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let primary = match &dialog {
                Dialog::Add { .. } => "Toevoegen",
                Dialog::Rename { .. } => "Opslaan",
                Dialog::Delete { .. } => CONFIRM_DELETE,
            };
            if helder::primary_button(ui, t, if matches!(dialog, Dialog::Delete { .. }) { Icon::Trash } else { Icon::Save }, primary).clicked() {
                ok = true;
            }
            helder::quiet(ui, t, |ui| {
                if ui.button(RichText::new("Annuleren").size(14.0).color(t.text)).clicked() {
                    close = true;
                }
            });
        });
    });
    if ok {
        let result = match &dialog {
            Dialog::Add { name, color } => app.engine.run("calendar.add", &json!({ "name": name, "color": color })),
            Dialog::Rename { id, name } => app.engine.run("calendar.rename", &json!({ "id": id, "name": name })),
            Dialog::Delete { id, .. } => app.engine.run("calendar.delete", &json!({ "id": id })),
        };
        match result {
            Ok(()) => close = true,
            Err(e) => app.notice = Some(crate::Notice::warning(describe(&e))),
        }
    }
    if !close {
        app.calendar.dialog = Some(dialog);
    }
}

fn name_too_long(dialog: &Dialog) -> bool {
    match dialog {
        Dialog::Add { name, .. } | Dialog::Rename { name, .. } => name.chars().count() > MAX_CALENDAR_NAME,
        Dialog::Delete { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_calendar_colour_stands_out_against_the_window_and_carries_its_tick() {
        for t in [Tokens::light(), Tokens::dark()] {
            for (name, c) in CALENDAR_COLORS {
                let ratio = crate::theme::contrast(rgb(c), t.window);
                assert!(ratio >= 3.0, "{name} on the window, dark {}: {ratio}", t.dark);
                let ratio = crate::theme::contrast(rgb(c), t.card);
                assert!(ratio >= 3.0, "{name} on a card, dark {}: {ratio}", t.dark);
            }
        }
        for (name, c) in CALENDAR_COLORS {
            let ratio = crate::theme::contrast(TICK, rgb(c));
            assert!(ratio >= 3.0, "the tick on {name}: {ratio}");
        }
    }
}
