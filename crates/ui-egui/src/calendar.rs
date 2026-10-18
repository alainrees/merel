//! The calendar module: month, week and day views over the session's appointments, and the
//! appointment window. Times are shown in the computer's time zone.

use chrono::{DateTime, Datelike, Days, Local, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use egui::{Align2, Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use mailcraft_engine::CommandError;
use mailcraft_model::Appointment;

use crate::{MailApp, Tokens};

pub const MONTHS: [&str; 12] =
    ["januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november", "december"];
pub const WEEKDAYS: [&str; 7] = ["maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag", "zondag"];
const WEEKDAYS_SHORT: [&str; 7] = ["ma", "di", "wo", "do", "vr", "za", "zo"];

/// Pixels per hour in the week and day views.
const HOUR_HEIGHT: f32 = 44.0;
const DATE_FORMAT: &str = "%d-%m-%Y";
const TIME_FORMAT: &str = "%H:%M";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Day,
    Week,
    Month,
}

/// What the calendar module remembers between frames.
pub struct State {
    pub view: View,
    /// The day the view is built around.
    pub focus: NaiveDate,
    pub editor: Option<Editor>,
}

impl Default for State {
    fn default() -> Self {
        State { view: View::Month, focus: today(), editor: None }
    }
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn month_name(m: u32) -> &'static str {
    MONTHS.get(m.saturating_sub(1) as usize).copied().unwrap_or("")
}

fn weekday_name(d: NaiveDate) -> &'static str {
    WEEKDAYS[d.weekday().num_days_from_monday() as usize]
}

/// The Unix time of a local wall-clock moment; on a DST gap, the earlier reading.
pub fn to_timestamp(dt: NaiveDateTime) -> Option<i64> {
    Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp())
}

/// The local wall-clock reading of a Unix time.
pub fn to_local(ts: i64) -> NaiveDateTime {
    DateTime::from_timestamp(ts, 0).map(|d| d.with_timezone(&Local).naive_local()).unwrap_or_default()
}

fn midnight(d: NaiveDate) -> Option<i64> {
    to_timestamp(d.and_hms_opt(0, 0, 0)?)
}

/// The Monday on or before `d`.
fn week_start(d: NaiveDate) -> NaiveDate {
    d.checked_sub_days(Days::new(u64::from(d.weekday().num_days_from_monday()))).unwrap_or(d)
}

fn add_days(d: NaiveDate, n: i64) -> NaiveDate {
    let moved = if n >= 0 { d.checked_add_days(Days::new(n.unsigned_abs())) } else { d.checked_sub_days(Days::new(n.unsigned_abs())) };
    moved.unwrap_or(d)
}

fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
    let moved = if n >= 0 { d.checked_add_months(Months::new(n.unsigned_abs())) } else { d.checked_sub_months(Months::new(n.unsigned_abs())) };
    moved.unwrap_or(d)
}

fn hm(ts: i64) -> String {
    to_local(ts).format(TIME_FORMAT).to_string()
}

/// The appointment window's fields, as text the person edits.
pub struct Editor {
    pub id: Option<String>,
    /// The calendar a new appointment goes to; `None` for the first one.
    pub calendar_id: Option<String>,
    pub title: String,
    pub location: String,
    pub all_day: bool,
    pub start_date: String,
    pub start_time: String,
    pub end_date: String,
    pub end_time: String,
    pub notes: String,
    pub error: Option<String>,
}

impl Editor {
    /// A new one-hour appointment starting at `start`.
    pub fn new_at(start: NaiveDateTime) -> Self {
        let end = start.checked_add_signed(chrono::TimeDelta::hours(1)).unwrap_or(start);
        Editor {
            id: None,
            calendar_id: None,
            title: String::new(),
            location: String::new(),
            all_day: false,
            start_date: start.format(DATE_FORMAT).to_string(),
            start_time: start.format(TIME_FORMAT).to_string(),
            end_date: end.format(DATE_FORMAT).to_string(),
            end_time: end.format(TIME_FORMAT).to_string(),
            notes: String::new(),
            error: None,
        }
    }

    pub fn from_appointment(a: &Appointment) -> Self {
        let start = to_local(a.start);
        // An all-day appointment ends at the next midnight; show its last day instead.
        let end = if a.all_day { to_local(a.end - 1) } else { to_local(a.end) };
        Editor {
            id: Some(a.id.clone()),
            calendar_id: Some(a.calendar_id.clone()),
            title: a.title.clone(),
            location: a.location.clone(),
            all_day: a.all_day,
            start_date: start.format(DATE_FORMAT).to_string(),
            start_time: start.format(TIME_FORMAT).to_string(),
            end_date: end.format(DATE_FORMAT).to_string(),
            end_time: end.format(TIME_FORMAT).to_string(),
            notes: a.notes.clone(),
            error: None,
        }
    }

    /// The `appointment.save` parameters, or what is wrong with the fields.
    pub fn params(&self) -> Result<serde_json::Value, String> {
        let date = |s: &str, what: &str| {
            NaiveDate::parse_from_str(s.trim(), DATE_FORMAT).map_err(|_| format!("Vul een geldige {what} in, bijvoorbeeld 18-10-2026."))
        };
        let time = |s: &str, what: &str| {
            NaiveTime::parse_from_str(s.trim(), TIME_FORMAT).map_err(|_| format!("Vul een geldige {what} in, bijvoorbeeld 14:30."))
        };
        let start_date = date(&self.start_date, "begindatum")?;
        let end_date = date(&self.end_date, "einddatum")?;
        let (start, end) = if self.all_day {
            let end_date = end_date.checked_add_days(Days::new(1)).ok_or("De einddatum ligt te ver weg.")?;
            (start_date.and_time(NaiveTime::MIN), end_date.and_time(NaiveTime::MIN))
        } else {
            (start_date.and_time(time(&self.start_time, "begintijd")?), end_date.and_time(time(&self.end_time, "eindtijd")?))
        };
        let start = to_timestamp(start).ok_or("De begintijd bestaat niet in deze tijdzone.")?;
        let end = to_timestamp(end).ok_or("De eindtijd bestaat niet in deze tijdzone.")?;
        if self.title.trim().is_empty() {
            return Err("Vul een titel in.".into());
        }
        if end <= start {
            return Err("Het einde ligt voor het begin.".into());
        }
        let mut p = json!({
            "title": self.title,
            "start": start,
            "end": end,
            "all_day": self.all_day,
            "location": self.location,
            "notes": self.notes,
        });
        if let Some(id) = &self.id {
            p["id"] = json!(id);
        }
        if let Some(c) = &self.calendar_id {
            p["calendar_id"] = json!(c);
        }
        Ok(p)
    }
}

impl MailApp {
    /// Opens the window for a new appointment on the focused day at `hour`.
    pub fn new_appointment(&mut self, date: NaiveDate, hour: u32) {
        let start = date.and_hms_opt(hour.min(23), 0, 0).unwrap_or_else(|| date.and_time(NaiveTime::MIN));
        self.calendar.editor = Some(Editor::new_at(start));
    }

    /// Selects an appointment and opens it in the window.
    pub fn open_appointment(&mut self, id: &str) {
        self.run("appointment.select", json!({ "id": id }));
        if let Some(a) = self.engine.session.appointment(id) {
            self.calendar.editor = Some(Editor::from_appointment(a));
        }
    }
}

/// The title above the view: "oktober 2026", "19 – 25 oktober 2026", "maandag 19 oktober 2026".
fn period_title(view: View, focus: NaiveDate) -> String {
    match view {
        View::Month => format!("{} {}", month_name(focus.month()), focus.year()),
        View::Week => {
            let a = week_start(focus);
            let b = add_days(a, 6);
            if a.month() == b.month() {
                format!("{} – {} {} {}", a.day(), b.day(), month_name(b.month()), b.year())
            } else {
                format!("{} {} – {} {} {}", a.day(), month_name(a.month()), b.day(), month_name(b.month()), b.year())
            }
        }
        View::Day => format!("{} {} {} {}", weekday_name(focus), focus.day(), month_name(focus.month()), focus.year()),
    }
}

fn step(view: View, focus: NaiveDate, n: i64) -> NaiveDate {
    match view {
        View::Month => add_months(focus, n as i32),
        View::Week => add_days(focus, 7 * n),
        View::Day => add_days(focus, n),
    }
}

/// The left pane: a small month to navigate with, and the calendars.
fn side_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("calendar-side")
        .default_size(240.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(Margin::symmetric(10, 10)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            let focus = app.calendar.focus;
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("◀").color(t.text)).frame(false)).on_hover_text("Vorige maand").clicked() {
                    app.calendar.focus = add_months(focus, -1);
                }
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(format!("{} {}", month_name(focus.month()), focus.year())).strong().color(t.text));
                });
            });
            // The next-month button sits at the right; egui lays out left to right, so it comes last.
            let r = ui.put(
                Rect::from_min_size(pos2(ui.max_rect().right() - 24.0, ui.max_rect().top()), vec2(24.0, 20.0)),
                egui::Button::new(RichText::new("▶").color(t.text)).frame(false),
            );
            if r.clicked() {
                app.calendar.focus = add_months(focus, 1);
            }
            ui.add_space(4.0);
            let cell = (ui.available_width() / 7.0).floor().clamp(20.0, 34.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for d in WEEKDAYS_SHORT {
                    let (rect, _) = ui.allocate_exact_size(vec2(cell, 18.0), Sense::hover());
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, d, FontId::proportional(11.0), t.text_dim);
                }
            });
            let first = focus.with_day(1).unwrap_or(focus);
            let mut day = week_start(first);
            let today = today();
            let mut pick = None;
            for _ in 0..6 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for _ in 0..7 {
                        let (rect, r) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("{} {} {}", day.day(), month_name(day.month()), day.year()),
                            )
                        });
                        let p = ui.painter();
                        if day == today {
                            p.circle_filled(rect.center(), cell / 2.0 - 2.0, t.accent);
                        } else if day == focus {
                            p.circle_stroke(rect.center(), cell / 2.0 - 2.0, Stroke::new(1.0, t.accent));
                        } else if r.hovered() {
                            p.circle_filled(rect.center(), cell / 2.0 - 2.0, t.hover);
                        }
                        let color = if day == today {
                            t.on_accent
                        } else if day.month() == focus.month() {
                            t.text
                        } else {
                            t.text_dim.gamma_multiply(0.6)
                        };
                        p.text(rect.center(), Align2::CENTER_CENTER, day.day().to_string(), FontId::proportional(12.0), color);
                        if r.clicked() {
                            pick = Some(day);
                        }
                        day = add_days(day, 1);
                    }
                });
            }
            if let Some(d) = pick {
                app.calendar.focus = d;
                if app.calendar.view == View::Month {
                    app.calendar.view = View::Day;
                }
            }
            ui.add_space(14.0);
            ui.label(RichText::new("Mijn agenda's").size(12.0).strong().color(t.text_dim));
            ui.add_space(4.0);
            for c in &app.engine.session.calendars {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(c.color[0], c.color[1], c.color[2]));
                    ui.label(RichText::new(&c.name).color(t.text));
                });
            }
        });
}

fn appointment_color(app: &MailApp, a: &Appointment, t: &Tokens) -> Color32 {
    app.engine.session.calendar(&a.calendar_id).map(|c| Color32::from_rgb(c.color[0], c.color[1], c.color[2])).unwrap_or(t.accent)
}

/// A clickable block for one appointment. Returns true when clicked.
fn chip(ui: &mut Ui, rect: Rect, label: &str, fill: Color32, selected: bool, t: &Tokens) -> bool {
    let r = ui.interact(rect, ui.make_persistent_id(("chip", rect.min.x as i32, rect.min.y as i32, label)), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let p = ui.painter();
    let alpha = if r.hovered() { 255 } else { 220 };
    p.rect_filled(rect, CornerRadius::same(3), Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), alpha));
    if selected {
        p.rect_stroke(rect, CornerRadius::same(3), Stroke::new(2.0, t.text), egui::StrokeKind::Inside);
    }
    let text_rect = rect.shrink2(vec2(5.0, 1.0));
    let galley = p.layout(label.to_string(), FontId::proportional(11.5), Color32::WHITE, text_rect.width());
    p.with_clip_rect(text_rect).galley(text_rect.left_top(), galley, Color32::WHITE);
    r.clicked()
}

fn month_view(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let focus = app.calendar.focus;
    let first = focus.with_day(1).unwrap_or(focus);
    let start = week_start(first);
    let today = today();
    let selected = app.engine.session.selected_appointment.clone();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let w = ui.available_width() / 7.0;
        for d in WEEKDAYS_SHORT {
            let (rect, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, d, FontId::proportional(12.0), t.text_dim);
        }
    });
    let avail = ui.available_size();
    let w = avail.x / 7.0;
    let h = (avail.y / 6.0).max(60.0);
    let mut open: Option<String> = None;
    let mut pick_day: Option<NaiveDate> = None;
    let mut day = start;
    for _ in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for _ in 0..7 {
                let (rect, r) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("{} {} {}", weekday_name(day), day.day(), month_name(day.month())),
                    )
                });
                let in_month = day.month() == focus.month();
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::ZERO, if in_month { t.reading } else { t.list });
                p.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                let number_pos = pos2(rect.right() - 14.0, rect.top() + 12.0);
                if day == today {
                    p.circle_filled(number_pos, 11.0, t.accent);
                }
                let color = if day == today {
                    t.on_accent
                } else if in_month {
                    t.text
                } else {
                    t.text_dim
                };
                p.text(number_pos, Align2::CENTER_CENTER, day.day().to_string(), FontId::proportional(12.5), color);
                if r.clicked() {
                    pick_day = Some(day);
                }
                if let (Some(from), Some(to)) = (midnight(day), midnight(add_days(day, 1))) {
                    let items: Vec<Appointment> = app.engine.session.appointments_between(from, to).into_iter().cloned().collect();
                    let room = ((rect.height() - 26.0) / 20.0).floor().max(0.0) as usize;
                    let shown = if items.len() > room { room.saturating_sub(1) } else { items.len() };
                    for (i, a) in items.iter().take(shown).enumerate() {
                        let y = rect.top() + 24.0 + i as f32 * 20.0;
                        let chip_rect = Rect::from_min_size(pos2(rect.left() + 3.0, y), vec2(rect.width() - 6.0, 18.0));
                        let label = if a.all_day { a.title.clone() } else { format!("{} {}", hm(a.start), a.title) };
                        if chip(ui, chip_rect, &label, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), t) {
                            open = Some(a.id.clone());
                        }
                    }
                    if items.len() > shown {
                        let y = rect.top() + 24.0 + shown as f32 * 20.0;
                        ui.painter().text(
                            pos2(rect.left() + 6.0, y + 9.0),
                            Align2::LEFT_CENTER,
                            format!("+{} meer", items.len() - shown),
                            FontId::proportional(11.0),
                            t.text_dim,
                        );
                    }
                }
                day = add_days(day, 1);
            }
        });
    }
    if let Some(id) = open {
        app.open_appointment(&id);
    } else if let Some(d) = pick_day {
        app.calendar.focus = d;
        app.calendar.view = View::Day;
    }
}

/// The week and day views: an all-day strip, then the hours.
fn time_grid(app: &mut MailApp, ui: &mut Ui, t: &Tokens, days: i64) {
    let first = if days == 7 { week_start(app.calendar.focus) } else { app.calendar.focus };
    let today = today();
    let selected = app.engine.session.selected_appointment.clone();
    let gutter = 48.0;
    let mut open: Option<String> = None;
    let mut new_at: Option<(NaiveDate, u32)> = None;
    let col_w = ((ui.available_width() - gutter) / days as f32).max(40.0);
    // Day headers.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_exact_size(vec2(gutter, 36.0), Sense::hover());
        for i in 0..days {
            let d = add_days(first, i);
            let (rect, _) = ui.allocate_exact_size(vec2(col_w, 36.0), Sense::hover());
            let p = ui.painter();
            let label = format!("{} {}", WEEKDAYS_SHORT[d.weekday().num_days_from_monday() as usize], d.day());
            let color = if d == today { t.accent_text } else { t.text };
            p.text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(13.0), color);
            if d == today {
                p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, t.accent));
            }
        }
    });
    // All-day strip.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let strip_h = 26.0;
        let (g, _) = ui.allocate_exact_size(vec2(gutter, strip_h), Sense::hover());
        ui.painter().text(g.right_center() - vec2(4.0, 0.0), Align2::RIGHT_CENTER, "hele dag", FontId::proportional(10.5), t.text_dim);
        for i in 0..days {
            let d = add_days(first, i);
            let (rect, _) = ui.allocate_exact_size(vec2(col_w, strip_h), Sense::hover());
            ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            if let (Some(from), Some(to)) = (midnight(d), midnight(add_days(d, 1))) {
                let items: Vec<Appointment> = app.engine.session.appointments_between(from, to).into_iter().filter(|a| a.all_day).cloned().collect();
                if let Some(a) = items.first() {
                    let label = if items.len() > 1 { format!("{} (+{})", a.title, items.len() - 1) } else { a.title.clone() };
                    let chip_rect = rect.shrink2(vec2(2.0, 3.0));
                    if chip(ui, chip_rect, &label, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), t) {
                        open = Some(a.id.clone());
                    }
                }
            }
        }
    });
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("hours").show(ui, |ui| {
        let (grid, _) = ui.allocate_exact_size(vec2(gutter + col_w * days as f32, HOUR_HEIGHT * 24.0), Sense::hover());
        let p = ui.painter();
        for hour in 0..24 {
            let y = grid.top() + hour as f32 * HOUR_HEIGHT;
            p.line_segment([pos2(grid.left() + gutter, y), pos2(grid.right(), y)], Stroke::new(1.0, t.border));
            p.text(pos2(grid.left() + gutter - 6.0, y + 2.0), Align2::RIGHT_TOP, format!("{hour:02}:00"), FontId::proportional(11.0), t.text_dim);
        }
        for i in 0..days {
            let d = add_days(first, i);
            let x = grid.left() + gutter + i as f32 * col_w;
            let column = Rect::from_min_size(pos2(x, grid.top()), vec2(col_w, grid.height()));
            ui.painter().line_segment([column.left_top(), column.left_bottom()], Stroke::new(1.0, t.border));
            let r = ui.interact(column, ui.make_persistent_id(("column", i)), Sense::click());
            r.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} {} {}", weekday_name(d), d.day(), month_name(d.month())))
            });
            if r.clicked()
                && let Some(pos) = r.interact_pointer_pos()
            {
                let hour = ((pos.y - grid.top()) / HOUR_HEIGHT).floor().clamp(0.0, 23.0) as u32;
                new_at = Some((d, hour));
            }
            let (Some(from), Some(to)) = (midnight(d), midnight(add_days(d, 1))) else { continue };
            let items: Vec<Appointment> = app.engine.session.appointments_between(from, to).into_iter().filter(|a| !a.all_day).cloned().collect();
            for a in &items {
                let top = ((a.start.max(from) - from) as f32 / 3600.0) * HOUR_HEIGHT;
                let bottom = ((a.end.min(to) - from) as f32 / 3600.0) * HOUR_HEIGHT;
                let block =
                    Rect::from_min_max(pos2(x + 2.0, grid.top() + top + 1.0), pos2(x + col_w - 2.0, grid.top() + bottom.max(top + 18.0) - 1.0));
                let label = format!("{} {}\n{}", hm(a.start), hm(a.end), a.title);
                if chip(ui, block, &label, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), t) {
                    open = Some(a.id.clone());
                }
            }
            if d == today {
                let now = Local::now().naive_local().time();
                let y = grid.top() + (now.hour() as f32 + now.minute() as f32 / 60.0) * HOUR_HEIGHT;
                ui.painter().line_segment([pos2(x, y), pos2(x + col_w, y)], Stroke::new(2.0, t.flag));
            }
        }
    });
    if let Some(id) = open {
        app.open_appointment(&id);
    } else if let Some((d, hour)) = new_at {
        app.calendar.focus = d;
        app.new_appointment(d, hour);
    }
}

/// The calendar module's panes; the ribbon and the window are drawn by the caller.
pub fn show(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    side_pane(app, ui, t);
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading).inner_margin(Margin::same(12))).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("◀").color(t.text)).min_size(vec2(28.0, 26.0))).on_hover_text("Vorige").clicked() {
                app.calendar.focus = step(app.calendar.view, app.calendar.focus, -1);
            }
            if ui.add(egui::Button::new(RichText::new("▶").color(t.text)).min_size(vec2(28.0, 26.0))).on_hover_text("Volgende").clicked() {
                app.calendar.focus = step(app.calendar.view, app.calendar.focus, 1);
            }
            ui.add_space(8.0);
            ui.label(RichText::new(period_title(app.calendar.view, app.calendar.focus)).size(18.0).strong().color(t.text));
        });
        ui.add_space(8.0);
        match app.calendar.view {
            View::Month => month_view(app, ui, t),
            View::Week => time_grid(app, ui, t, 7),
            View::Day => time_grid(app, ui, t, 1),
        }
    });
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, width: f32) {
    ui.horizontal(|ui| {
        ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new(label).color(t.text)));
        ui.add(egui::TextEdit::singleline(value).desired_width(width));
    });
}

/// The appointment window, when one is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let calendars: Vec<(String, String)> = app.engine.session.calendars.iter().map(|c| (c.id.clone(), c.name.clone())).collect();
    let Some(editor) = app.calendar.editor.as_mut() else { return };
    let mut save = false;
    let mut delete = false;
    let mut close = false;
    let title = if editor.id.is_some() { "Afspraak" } else { "Nieuwe afspraak" };
    egui::Window::new(title).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
        field(ui, t, "Titel", &mut editor.title, 300.0);
        field(ui, t, "Locatie", &mut editor.location, 300.0);
        // Which calendar: a choice for a new appointment when there is one, a fact otherwise.
        if calendars.len() > 1 {
            let current = editor.calendar_id.clone().or_else(|| calendars.first().map(|(id, _)| id.clone()));
            let name = calendars.iter().find(|(id, _)| Some(id) == current.as_ref()).map(|(_, n)| n.as_str()).unwrap_or("");
            ui.horizontal(|ui| {
                ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Agenda").color(t.text)));
                if editor.id.is_some() {
                    ui.label(RichText::new(name).color(t.text_dim));
                } else {
                    egui::ComboBox::from_id_salt("appointment-calendar").selected_text(name).width(300.0).show_ui(ui, |ui| {
                        for (id, n) in &calendars {
                            ui.selectable_value(&mut editor.calendar_id, Some(id.clone()), n);
                        }
                    });
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add_space(94.0);
            ui.checkbox(&mut editor.all_day, "Hele dag");
        });
        ui.horizontal(|ui| {
            ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Begin").color(t.text)));
            ui.add(egui::TextEdit::singleline(&mut editor.start_date).desired_width(90.0));
            if !editor.all_day {
                ui.add(egui::TextEdit::singleline(&mut editor.start_time).desired_width(50.0));
            }
        });
        ui.horizontal(|ui| {
            ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Einde").color(t.text)));
            ui.add(egui::TextEdit::singleline(&mut editor.end_date).desired_width(90.0));
            if !editor.all_day {
                ui.add(egui::TextEdit::singleline(&mut editor.end_time).desired_width(50.0));
            }
        });
        ui.horizontal(|ui| {
            ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Notities").color(t.text)));
            ui.add(egui::TextEdit::multiline(&mut editor.notes).desired_width(300.0).desired_rows(4));
        });
        if let Some(e) = &editor.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            save = ui.add(egui::Button::new(RichText::new("Opslaan").color(t.on_accent)).fill(t.accent).min_size(vec2(90.0, 26.0))).clicked();
            if editor.id.is_some() {
                delete = ui.add(egui::Button::new("Verwijderen").min_size(vec2(90.0, 26.0))).clicked();
            }
            close = ui.add(egui::Button::new("Annuleren").min_size(vec2(90.0, 26.0))).clicked();
        });
    });
    if save {
        match editor.params() {
            Ok(params) => match app.engine.run("appointment.save", &params) {
                Ok(()) => close = true,
                Err(e) => editor.error = Some(describe(&e)),
            },
            Err(e) => editor.error = Some(e),
        }
    }
    if delete && let Some(id) = editor.id.clone() {
        match app.engine.run("appointment.delete", &json!({ "id": id })) {
            Ok(()) => close = true,
            Err(e) => editor.error = Some(describe(&e)),
        }
    }
    if close {
        app.calendar.editor = None;
    }
}

fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("title") => "Vul een titel in.".into(),
        CommandError::Param("end") => "Het einde ligt voor het begin.".into(),
        CommandError::Store(m) => format!("Opslaan is mislukt: {m}"),
        CommandError::NotFound => "Deze afspraak bestaat niet meer.".into(),
        e => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn titles_and_steps_follow_the_view() {
        let focus = d(2026, 10, 21);
        assert_eq!(period_title(View::Month, focus), "oktober 2026");
        assert_eq!(period_title(View::Week, focus), "19 – 25 oktober 2026");
        assert_eq!(period_title(View::Week, d(2026, 10, 30)), "26 oktober – 1 november 2026");
        assert_eq!(period_title(View::Day, focus), "woensdag 21 oktober 2026");
        assert_eq!(step(View::Month, d(2026, 1, 31), 1), d(2026, 2, 28));
        assert_eq!(step(View::Week, focus, -1), d(2026, 10, 14));
        assert_eq!(week_start(d(2026, 10, 25)), d(2026, 10, 19));
    }

    #[test]
    fn editor_fields_round_trip_through_params() {
        let mut e = Editor::new_at(d(2026, 10, 21).and_hms_opt(14, 0, 0).unwrap());
        assert_eq!((e.start_date.as_str(), e.start_time.as_str(), e.end_time.as_str()), ("21-10-2026", "14:00", "15:00"));
        assert_eq!(e.params(), Err("Vul een titel in.".to_string()));
        e.title = "Lunch".into();
        let p = e.params().unwrap();
        let start = p["start"].as_i64().unwrap();
        assert_eq!(p["end"].as_i64().unwrap() - start, 3600);
        assert_eq!(to_local(start), d(2026, 10, 21).and_hms_opt(14, 0, 0).unwrap());
        e.end_time = "13:00".into();
        assert_eq!(e.params(), Err("Het einde ligt voor het begin.".to_string()));
        e.start_date = "21/10/2026".into();
        assert!(e.params().unwrap_err().contains("begindatum"));

        let mut whole = Editor::new_at(d(2026, 10, 21).and_hms_opt(9, 0, 0).unwrap());
        whole.title = "Studiedag".into();
        whole.all_day = true;
        let p = whole.params().unwrap();
        assert_eq!(p["end"].as_i64().unwrap() - p["start"].as_i64().unwrap(), 86_400);
        let a = Appointment {
            id: "x".into(),
            calendar_id: "local".into(),
            title: "Studiedag".into(),
            start: p["start"].as_i64().unwrap(),
            end: p["end"].as_i64().unwrap(),
            all_day: true,
            location: String::new(),
            notes: String::new(),
        };
        let back = Editor::from_appointment(&a);
        assert_eq!((back.start_date.as_str(), back.end_date.as_str()), ("21-10-2026", "21-10-2026"));
    }
}
