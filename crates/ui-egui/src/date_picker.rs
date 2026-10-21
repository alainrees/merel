//! A calendar button beside a date field, as Outlook has it: a small month to pick the day
//! from, with buttons for the previous month, today and the next month. The field stays
//! typable as dag-maand-jaar; the month is a quicker way to fill it.

use crate::i18n::tr;
use chrono::{Datelike, NaiveDate};
use egui::{Align2, FontId, Response, RichText, Sense, Stroke, Ui, vec2};

use crate::Tokens;
use crate::calendar::{add_days, add_months, month_name, nav_button, parse_date, today, week_start};
use crate::helder::{self, Icon};

const DATE_FORMAT: &str = "%d-%m-%Y";
const CELL: f32 = 32.0;

/// A date field: what `helder::underline_field` draws, followed by the calendar button.
/// Returns the day picked from the calendar, if one was picked this frame.
pub fn field(ui: &mut Ui, t: &Tokens, text: &mut String, hint: &str, width: f32, label: &str) -> Option<NaiveDate> {
    helder::underline_field(ui, t, text, hint, width, false);
    button(ui, t, text, label)
}

/// The calendar button for `text`, a day as dag-maand-jaar. `label` names it for a screen
/// reader and as its tooltip, such as "Begindatum kiezen". Picking a day fills `text` and
/// returns that day.
pub fn button(ui: &mut Ui, t: &Tokens, text: &mut String, label: &str) -> Option<NaiveDate> {
    let r = helder::icon_button(ui, t, Icon::Calendar, label);
    let shown_id = r.id.with("shown");
    if r.clicked() {
        // Each time it opens, it shows the month of the day in the field, else this month.
        let day = parse_date(text).unwrap_or_else(today);
        ui.data_mut(|d| d.insert_temp(shown_id, day.with_day(1).unwrap_or(day)));
    }
    let mut picked = None;
    egui::Popup::from_toggle_button_response(&r)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(helder::window_frame(ui.ctx(), t))
        .show(|ui| {
            let selected = parse_date(text);
            let mut shown: NaiveDate = ui.data(|d| d.get_temp(shown_id)).unwrap_or_else(|| selected.unwrap_or_else(today));
            if let Some(day) = month(ui, t, &mut shown, selected) {
                picked = Some(day);
                ui.close();
            }
            ui.data_mut(|d| d.insert_temp(shown_id, shown));
        });
    if let Some(day) = picked {
        *text = day.format(DATE_FORMAT).to_string();
    }
    picked
}

/// The month `shown`, with its header and navigation; returns the day clicked.
fn month(ui: &mut Ui, t: &Tokens, shown: &mut NaiveDate, selected: Option<NaiveDate>) -> Option<NaiveDate> {
    let width = CELL * 7.0;
    ui.set_width(width);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} {}", month_name(shown.month()), shown.year())).size(15.0).strong().color(t.text));
        helder::right(ui, |ui| {
            helder::quiet(ui, t, |ui| {
                if nav_button(ui, t, Icon::ChevronRight, &tr("calendar-next-month")).clicked() {
                    *shown = add_months(*shown, 1);
                }
                if nav_button(ui, t, Icon::CalendarDay, &tr("calendar-today")).clicked() {
                    *shown = today();
                }
                if nav_button(ui, t, Icon::ChevronLeft, &tr("calendar-previous-month")).clicked() {
                    *shown = add_months(*shown, -1);
                }
            });
        });
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for d in (0..7).map(crate::i18n::weekday_letter) {
            let (rect, _) = ui.allocate_exact_size(vec2(CELL, 20.0), Sense::hover());
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, d, FontId::proportional(12.0), t.text_dim);
        }
    });
    let first = shown.with_day(1).unwrap_or(*shown);
    let mut day = week_start(first);
    let now = today();
    let mut pick = None;
    for _ in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for _ in 0..7 {
                if let Some(d) = day_cell(ui, t, day, shown.month(), Some(day) == selected, day == now) {
                    pick = Some(d);
                }
                day = add_days(day, 1);
            }
        });
    }
    pick
}

/// One day of the month; returns it when clicked.
fn day_cell(ui: &mut Ui, t: &Tokens, day: NaiveDate, month: u32, selected: bool, is_today: bool) -> Option<NaiveDate> {
    let (rect, r): (egui::Rect, Response) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, format!("{} {} {}", day.day(), month_name(day.month()), day.year()))
    });
    let p = ui.painter();
    let radius = CELL / 2.0 - 2.0;
    if selected {
        p.circle_filled(rect.center(), radius, t.accent);
    } else if r.hovered() {
        p.circle_filled(rect.center(), radius, t.hover);
    }
    if is_today && !selected {
        p.circle_stroke(rect.center(), radius, Stroke::new(1.0, t.accent));
    }
    let color = if selected {
        t.on_accent
    } else if is_today {
        t.accent_text
    } else if day.month() == month {
        t.text
    } else {
        t.text_disabled
    };
    p.text(rect.center(), Align2::CENTER_CENTER, day.day().to_string(), FontId::proportional(13.0), color);
    r.clicked().then_some(day)
}
