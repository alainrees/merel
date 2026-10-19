//! The calendar module: month, week and day views over the session's appointments, and the
//! appointment window. Times are shown in the computer's time zone. A recurring appointment
//! shows every occurrence; opening one opens the series, with a button to skip just that time.
//! A search box above the view lists the matching appointments of the past year and the two
//! years ahead instead of the view, until it is cleared. A block can be dragged to another day
//! (month view) or half hour (week and day views); a series cannot, it is changed in its window.

use chrono::{DateTime, Datelike, Days, Local, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use egui::{Align2, Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use mailcraft_engine::{CommandError, tasks::TaskFilter};
use mailcraft_model::{Appointment, Frequency, Recurrence, Task, describe_recurrence};

use crate::{MailApp, Module, Tokens};

pub const MONTHS: [&str; 12] =
    ["januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november", "december"];
pub const WEEKDAYS: [&str; 7] = ["maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag", "zondag"];
const WEEKDAYS_SHORT: [&str; 7] = ["ma", "di", "wo", "do", "vr", "za", "zo"];
/// Marks an occurrence of a recurring appointment in the views.
const REPEAT_MARK: &str = "🔁";
/// The search box's widget id, so Ctrl+E can put the cursor there.
pub(crate) const SEARCH_BOX: &str = "calendar_search_box";
/// How far a search looks: this many days back and ahead of today.
const SEARCH_BACK_DAYS: i64 = 365;
const SEARCH_AHEAD_DAYS: i64 = 731;

/// Pixels per hour in the week and day views.
const HOUR_HEIGHT: f32 = 44.0;
const DATE_FORMAT: &str = "%d-%m-%Y";
const TIME_FORMAT: &str = "%H:%M";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Day,
    /// Monday to Friday, Outlook's "Werkweek".
    WorkWeek,
    Week,
    Month,
}

/// What the calendar module remembers between frames.
pub struct State {
    pub view: View,
    /// The day the view is built around.
    pub focus: NaiveDate,
    /// What the search box holds; while it has words, the results list replaces the view.
    pub query: String,
    pub editor: Option<Editor>,
    /// The appointment whose bottom edge is being dragged, with the end it would get.
    pub resizing: Option<Resizing>,
}

/// An appointment being stretched or shortened by its bottom edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resizing {
    pub id: String,
    pub end: i64,
}

impl Default for State {
    fn default() -> Self {
        State { view: View::Month, focus: today(), query: String::new(), editor: None, resizing: None }
    }
}

impl State {
    /// Whether the results list is shown instead of the view.
    pub fn is_searching(&self) -> bool {
        !self.query.trim().is_empty()
    }
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

pub(crate) fn month_name(m: u32) -> &'static str {
    MONTHS.get(m.saturating_sub(1) as usize).copied().unwrap_or("")
}

pub(crate) fn weekday_name(d: NaiveDate) -> &'static str {
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

/// How a series ends, as the window asks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ends {
    Never,
    /// On the date in `Editor::until`.
    OnDate,
    /// After the number of times in `Editor::count`.
    AfterCount,
}

/// The appointment window's fields, as text the person edits.
pub struct Editor {
    pub id: Option<String>,
    /// The calendar a new appointment goes to; `None` for the first one.
    pub calendar_id: Option<String>,
    pub title: String,
    pub location: String,
    /// The addresses to invite, as typed; "Verzenden" then saves the appointment and mails them
    /// a meeting request from the calendar's account. Empty for an appointment of your own.
    pub attendees: String,
    pub all_day: bool,
    pub start_date: String,
    pub start_time: String,
    pub end_date: String,
    pub end_time: String,
    pub notes: String,
    /// `None` for a single appointment.
    pub repeat: Option<Frequency>,
    pub interval: String,
    /// Weekly: Monday to Sunday.
    pub weekdays: [bool; 7],
    pub ends: Ends,
    pub until: String,
    pub count: String,
    /// The start of the occurrence the window was opened from, for a recurring appointment.
    pub occurrence: Option<i64>,
    /// Minutes before the start a reminder comes; `None` for none.
    pub reminder: Option<u32>,
    pub error: Option<String>,
}

/// The reminder choices of the window: minutes before the start, and the label.
const REMINDERS: [(Option<u32>, &str); 10] = [
    (None, "Geen"),
    (Some(0), "Op het moment zelf"),
    (Some(5), "5 minuten ervoor"),
    (Some(10), "10 minuten ervoor"),
    (Some(15), "15 minuten ervoor"),
    (Some(30), "30 minuten ervoor"),
    (Some(60), "1 uur ervoor"),
    (Some(120), "2 uur ervoor"),
    (Some(1440), "1 dag ervoor"),
    (Some(10_080), "1 week ervoor"),
];

/// The label of a reminder setting, also one the list does not offer.
fn reminder_label(reminder: Option<u32>) -> String {
    match REMINDERS.iter().find(|(r, _)| *r == reminder) {
        Some((_, label)) => (*label).to_string(),
        None => format!("{} minuten ervoor", reminder.unwrap_or(0)),
    }
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
            attendees: String::new(),
            all_day: false,
            start_date: start.format(DATE_FORMAT).to_string(),
            start_time: start.format(TIME_FORMAT).to_string(),
            end_date: end.format(DATE_FORMAT).to_string(),
            end_time: end.format(TIME_FORMAT).to_string(),
            notes: String::new(),
            repeat: None,
            interval: "1".into(),
            weekdays: [false; 7],
            ends: Ends::Never,
            until: String::new(),
            count: "10".into(),
            occurrence: None,
            // Outlook's default: a quarter of an hour before.
            reminder: Some(15),
            error: None,
        }
    }

    /// The window for `a`; `occurrence` is the start of the occurrence that was clicked, when
    /// `a` repeats. The fields show the series itself, so its first occurrence.
    pub fn from_appointment(a: &Appointment, occurrence: Option<i64>) -> Self {
        let start = to_local(a.start);
        // An all-day appointment ends at the next midnight; show its last day instead.
        let end = if a.all_day { to_local(a.end - 1) } else { to_local(a.end) };
        let mut e = Editor::new_at(start);
        e.id = Some(a.id.clone());
        e.calendar_id = Some(a.calendar_id.clone());
        e.title = a.title.clone();
        e.location = a.location.clone();
        e.all_day = a.all_day;
        e.reminder = a.reminder;
        e.end_date = end.format(DATE_FORMAT).to_string();
        e.end_time = end.format(TIME_FORMAT).to_string();
        e.notes = a.notes.clone();
        if let Some(r) = &a.recurrence {
            e.repeat = Some(r.frequency);
            e.interval = r.interval.to_string();
            for d in &r.weekdays {
                if let Some(w) = e.weekdays.get_mut(*d as usize) {
                    *w = true;
                }
            }
            if r.weekdays.is_empty() {
                e.weekdays[start.weekday().num_days_from_monday() as usize] = true;
            }
            match (r.until, r.count) {
                (Some(u), _) => {
                    e.ends = Ends::OnDate;
                    e.until = to_local(u).format(DATE_FORMAT).to_string();
                }
                (None, Some(c)) => {
                    e.ends = Ends::AfterCount;
                    e.count = c.to_string();
                }
                (None, None) => e.ends = Ends::Never,
            }
            e.occurrence = occurrence;
        }
        e
    }

    /// The recurrence the fields describe, or what is wrong with them.
    fn recurrence(&self, start_date: NaiveDate) -> Result<Option<Recurrence>, String> {
        let Some(frequency) = self.repeat else { return Ok(None) };
        let interval: u32 = self
            .interval
            .trim()
            .parse()
            .ok()
            .filter(|n| (1..=mailcraft_model::MAX_INTERVAL).contains(n))
            .ok_or("Vul in om de hoeveel keer de afspraak terugkomt, van 1 tot 999.")?;
        let mut weekdays: Vec<u8> = self.weekdays.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i as u8).collect();
        if frequency == Frequency::Weekly && weekdays.is_empty() {
            weekdays.push(start_date.weekday().num_days_from_monday() as u8);
        }
        let (until, count) = match self.ends {
            Ends::Never => (None, None),
            Ends::OnDate => {
                let date = NaiveDate::parse_from_str(self.until.trim(), DATE_FORMAT)
                    .map_err(|_| "Vul een geldige einddatum voor de herhaling in, bijvoorbeeld 31-12-2026.".to_string())?;
                let next = date.checked_add_days(Days::new(1)).ok_or("De einddatum van de herhaling ligt te ver weg.")?;
                let last = midnight(next).ok_or("De einddatum van de herhaling bestaat niet in deze tijdzone.")? - 1;
                if date < start_date {
                    return Err("De herhaling eindigt voor de eerste keer.".into());
                }
                (Some(last), None)
            }
            Ends::AfterCount => {
                let n: u32 = self.count.trim().parse().ok().filter(|n| *n >= 1).ok_or("Vul in na hoeveel keer de herhaling stopt, minstens 1.")?;
                (None, Some(n))
            }
        };
        Ok(Some(Recurrence { frequency, interval, weekdays, until, count, skipped: Vec::new() }))
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
        let recurrence = self.recurrence(start_date)?;
        let mut p = json!({
            "title": self.title,
            "start": start,
            "end": end,
            "all_day": self.all_day,
            "location": self.location,
            "notes": self.notes,
            "recurrence": recurrence,
            "reminder": self.reminder,
        });
        if recurrence.is_some() {
            // The engine keeps the occurrences that were skipped; the window never lists them.
            if let Some(r) = p.get_mut("recurrence").and_then(|r| r.as_object_mut()) {
                r.remove("skipped");
            }
        }
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

    /// Moves a dragged appointment to `day`, at `time` when the drop named one; what went wrong
    /// becomes a notice.
    pub fn drop_appointment(&mut self, id: &str, day: NaiveDate, time: Option<NaiveTime>) {
        let Some(a) = self.engine.session.appointment(id) else { return };
        let Some(start) = dropped_start(a, day, time) else {
            self.notice = Some("Die tijd bestaat niet in deze tijdzone.".into());
            return;
        };
        if start == a.start {
            return;
        }
        self.notice = self.engine.run("appointment.move", &json!({ "id": id, "start": start })).err().map(|e| describe(&e));
    }

    /// A task was dropped on `day` (at `time` in the hour grid): the appointment window opens
    /// for it, filled in from the task, to be saved by the person.
    pub fn plan_task(&mut self, id: &str, day: NaiveDate, time: Option<NaiveTime>) {
        let Some(task) = self.engine.session.task(id) else { return };
        self.calendar.focus = day;
        self.calendar.editor = Some(planned(task, day, time));
    }

    /// The bottom edge of `id` was let go: the end it was dragged to becomes the appointment's,
    /// unless nothing changed. What went wrong becomes a notice.
    pub fn finish_resize(&mut self, id: &str) {
        let Some(r) = self.calendar.resizing.take() else { return };
        if r.id != id {
            return;
        }
        let Some(a) = self.engine.session.appointment(id) else { return };
        if r.end == a.end {
            return;
        }
        self.notice = self.engine.run("appointment.resize", &json!({ "id": id, "end": r.end })).err().map(|e| describe(&e));
    }

    /// Selects an appointment and opens it in the window; `start` is the occurrence that was
    /// clicked, which matters for a recurring one.
    pub fn open_appointment(&mut self, id: &str, start: i64) {
        self.run("appointment.select", json!({ "id": id }));
        if let Some(a) = self.engine.session.appointment(id) {
            self.calendar.editor = Some(Editor::from_appointment(a, Some(start)));
        }
    }
}

/// The title above the view: "oktober 2026", "19 – 25 oktober 2026" (19 – 23 for the work
/// week), "maandag 19 oktober 2026".
/// The first and last day the view shows: the day, the Monday to Friday or Sunday, or the month.
pub(crate) fn period(view: View, focus: NaiveDate) -> (NaiveDate, NaiveDate) {
    match view {
        View::Day => (focus, focus),
        View::WorkWeek => (week_start(focus), add_days(week_start(focus), 4)),
        View::Week => (week_start(focus), add_days(week_start(focus), 6)),
        View::Month => {
            let first = focus.with_day(1).unwrap_or(focus);
            (first, add_days(add_months(first, 1), -1))
        }
    }
}

pub(crate) fn period_title(view: View, focus: NaiveDate) -> String {
    match view {
        View::Month => format!("{} {}", month_name(focus.month()), focus.year()),
        View::Week | View::WorkWeek => {
            let a = week_start(focus);
            let b = add_days(a, if view == View::Week { 6 } else { 4 });
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
        View::Week | View::WorkWeek => add_days(focus, 7 * n),
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
            task_list(app, ui, t);
        });
}

/// How many open tasks the calendar's side pane lists.
const SIDE_TASKS: usize = 12;

/// The open tasks under the calendars, each a drag source: dropped on a day or an hour, a task
/// becomes a new appointment. A click opens the task in the tasks module.
fn task_list(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let tasks: Vec<Task> = app.engine.session.tasks_filtered(TaskFilter::Open).into_iter().take(SIDE_TASKS).cloned().collect();
    if tasks.is_empty() {
        return;
    }
    ui.add_space(14.0);
    ui.label(RichText::new("Taken").size(12.0).strong().color(t.text_dim))
        .on_hover_text("Sleep een taak naar een dag of uur om er een afspraak voor in te plannen.");
    ui.add_space(4.0);
    let mut pick = None;
    for task in &tasks {
        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &task.title));
        r.dnd_set_drag_payload(DraggedTask(task.id.clone()));
        if r.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(3), t.hover);
        }
        let p = ui.painter();
        p.rect_stroke(
            Rect::from_center_size(pos2(rect.left() + 9.0, rect.center().y), vec2(12.0, 12.0)),
            CornerRadius::same(2),
            Stroke::new(1.2, t.text_dim),
            egui::StrokeKind::Inside,
        );
        let text_rect = Rect::from_min_max(pos2(rect.left() + 22.0, rect.top()), rect.right_bottom());
        let galley = p.layout_no_wrap(task.title.clone(), FontId::proportional(12.5), t.text);
        p.with_clip_rect(text_rect).galley(pos2(text_rect.left(), rect.center().y - galley.size().y / 2.0), galley, t.text);
        if r.clicked() {
            pick = Some(task.id.clone());
        }
    }
    if let Some(id) = pick {
        app.run("task.select", json!({ "id": id }));
        app.module = Module::Tasks;
    }
}

/// The appointment window for `task` on `day`: at `time` for an hour when the drop named one
/// (a spot in the hour grid), else the whole day. The task's title and notes come along.
fn planned(task: &Task, day: NaiveDate, time: Option<NaiveTime>) -> Editor {
    let start = day.and_time(time.unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN)));
    Editor { title: task.title.clone(), notes: task.notes.clone(), all_day: time.is_none(), ..Editor::new_at(start) }
}

fn appointment_color(app: &MailApp, a: &Appointment, t: &Tokens) -> Color32 {
    app.engine.session.calendar(&a.calendar_id).map(|c| Color32::from_rgb(c.color[0], c.color[1], c.color[2])).unwrap_or(t.accent)
}

/// What is being dragged: one appointment, by id. A series is never dragged.
pub(crate) struct DraggedAppointment(String);

/// A task dragged from the side pane, by id, to plan it as an appointment.
pub(crate) struct DraggedTask(String);

/// What was let go over a drop target.
enum Dropped {
    Appointment(String),
    Task(String),
}

/// Whether something that can land on a day or an hour is held over `r`.
fn drop_hover(r: &egui::Response) -> bool {
    r.dnd_hover_payload::<DraggedAppointment>().is_some() || r.dnd_hover_payload::<DraggedTask>().is_some()
}

/// What was dropped on `r` this frame. The payload's type is looked at before it is taken:
/// taking it as the wrong type would throw it away, and a target accepts both kinds.
fn dropped(r: &egui::Response) -> Option<Dropped> {
    if egui::DragAndDrop::has_payload_of_type::<DraggedAppointment>(&r.ctx) {
        r.dnd_release_payload::<DraggedAppointment>().map(|a| Dropped::Appointment(a.0.clone()))
    } else if egui::DragAndDrop::has_payload_of_type::<DraggedTask>(&r.ctx) {
        r.dnd_release_payload::<DraggedTask>().map(|t| Dropped::Task(t.0.clone()))
    } else {
        None
    }
}

/// The drag payload for a block, when the appointment can be dragged.
fn draggable(a: &Appointment) -> Option<DraggedAppointment> {
    a.recurrence.is_none().then(|| DraggedAppointment(a.id.clone()))
}

/// Where a dropped appointment begins: on `day`, at `time` when the drop names one (a spot in
/// the hour grid), else at its own time of day; an all-day appointment always at midnight.
fn dropped_start(a: &Appointment, day: NaiveDate, time: Option<NaiveTime>) -> Option<i64> {
    if a.all_day {
        return midnight(day);
    }
    let time = time.unwrap_or_else(|| to_local(a.start).time());
    to_timestamp(day.and_time(time))
}

/// Where an appointment ends when its bottom edge is dragged to the half hour `time` on `day`:
/// the end of that half hour, and never less than half an hour after its start.
fn resized_end(a: &Appointment, day: NaiveDate, time: NaiveTime) -> Option<i64> {
    let end = to_timestamp(day.and_time(time))? + HALF_HOUR;
    Some(end.max(a.start + HALF_HOUR))
}

/// The height of the grab area along the bottom edge of a block in the hour grid.
const RESIZE_HANDLE: f32 = 7.0;
const HALF_HOUR: i64 = 1_800;

/// The half hour under `y` in an hour grid whose midnight is at `top`.
fn grid_time(y: f32, top: f32) -> NaiveTime {
    let halves = ((y - top) / HOUR_HEIGHT * 2.0).floor().clamp(0.0, 47.0) as u32;
    NaiveTime::from_hms_opt(halves / 2, (halves % 2) * 30, 0).unwrap_or(NaiveTime::MIN)
}

/// The chip text for an occurrence: the title, marked when it repeats.
fn chip_title(a: &Appointment) -> String {
    if a.recurrence.is_some() { format!("{REPEAT_MARK} {}", a.title) } else { a.title.clone() }
}

/// A clickable block for one appointment, a drag source when `drag` is given. Returns true
/// when clicked.
fn chip(ui: &mut Ui, rect: Rect, label: &str, fill: Color32, selected: bool, drag: Option<DraggedAppointment>, t: &Tokens) -> bool {
    let sense = if drag.is_some() { Sense::click_and_drag() } else { Sense::click() };
    let r = ui.interact(rect, ui.make_persistent_id(("chip", rect.min.x as i32, rect.min.y as i32, label)), sense);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if let Some(payload) = drag {
        r.dnd_set_drag_payload(payload);
    }
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
    let mut open: Option<(String, i64)> = None;
    let mut pick_day: Option<NaiveDate> = None;
    let mut drop: Option<(String, NaiveDate)> = None;
    let mut task_drop: Option<(String, NaiveDate)> = None;
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
                let drop_here = drop_hover(&r);
                match dropped(&r) {
                    Some(Dropped::Appointment(id)) => drop = Some((id, day)),
                    Some(Dropped::Task(id)) => task_drop = Some((id, day)),
                    None => {}
                }
                let p = ui.painter();
                p.rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    if drop_here {
                        t.hover
                    } else if in_month {
                        t.reading
                    } else {
                        t.list
                    },
                );
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
                    let items = app.engine.session.appointments_between(from, to);
                    let room = ((rect.height() - 26.0) / 20.0).floor().max(0.0) as usize;
                    let shown = if items.len() > room { room.saturating_sub(1) } else { items.len() };
                    for (i, a) in items.iter().take(shown).enumerate() {
                        let y = rect.top() + 24.0 + i as f32 * 20.0;
                        let chip_rect = Rect::from_min_size(pos2(rect.left() + 3.0, y), vec2(rect.width() - 6.0, 18.0));
                        let label = if a.all_day { chip_title(a) } else { format!("{} {}", hm(a.start), chip_title(a)) };
                        if chip(ui, chip_rect, &label, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), draggable(a), t) {
                            open = Some((a.id.clone(), a.start));
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
    if let Some((id, day)) = task_drop {
        app.plan_task(&id, day, None);
    } else if let Some((id, day)) = drop {
        app.drop_appointment(&id, day, None);
    } else if let Some((id, start)) = open {
        app.open_appointment(&id, start);
    } else if let Some(d) = pick_day {
        app.calendar.focus = d;
        app.calendar.view = View::Day;
    }
}

/// The week and day views: an all-day strip, then the hours.
fn time_grid(app: &mut MailApp, ui: &mut Ui, t: &Tokens, days: i64) {
    let first = if days > 1 { week_start(app.calendar.focus) } else { app.calendar.focus };
    let today = today();
    let selected = app.engine.session.selected_appointment.clone();
    let gutter = 48.0;
    let mut open: Option<(String, i64)> = None;
    let mut new_at: Option<(NaiveDate, u32)> = None;
    let mut drop: Option<(String, NaiveDate, Option<NaiveTime>)> = None;
    let mut task_drop: Option<(String, NaiveDate, Option<NaiveTime>)> = None;
    let mut resize: Option<Resizing> = None;
    let mut resized: Option<String> = None;
    let resizing = app.calendar.resizing.clone();
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
            let (rect, r) = ui.allocate_exact_size(vec2(col_w, strip_h), Sense::hover());
            if drop_hover(&r) {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, t.hover);
            }
            match dropped(&r) {
                Some(Dropped::Appointment(id)) => drop = Some((id, d, None)),
                Some(Dropped::Task(id)) => task_drop = Some((id, d, None)),
                None => {}
            }
            ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            if let (Some(from), Some(to)) = (midnight(d), midnight(add_days(d, 1))) {
                let items: Vec<Appointment> = app.engine.session.appointments_between(from, to).into_iter().filter(|a| a.all_day).collect();
                if let Some(a) = items.first() {
                    let label = if items.len() > 1 { format!("{} (+{})", chip_title(a), items.len() - 1) } else { chip_title(a) };
                    let chip_rect = rect.shrink2(vec2(2.0, 3.0));
                    if chip(ui, chip_rect, &label, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), draggable(a), t) {
                        open = Some((a.id.clone(), a.start));
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
            let pointer = ui.input(|i| i.pointer.latest_pos());
            if drop_hover(&r)
                && let Some(pos) = pointer
            {
                // The half hour the block would land in.
                let tm = grid_time(pos.y, grid.top());
                let y = grid.top() + (tm.hour() as f32 + tm.minute() as f32 / 60.0) * HOUR_HEIGHT;
                ui.painter().rect_filled(Rect::from_min_size(pos2(x, y), vec2(col_w, HOUR_HEIGHT / 2.0)), CornerRadius::ZERO, t.hover);
            }
            if let Some(pos) = pointer {
                match dropped(&r) {
                    Some(Dropped::Appointment(id)) => drop = Some((id, d, Some(grid_time(pos.y, grid.top())))),
                    Some(Dropped::Task(id)) => task_drop = Some((id, d, Some(grid_time(pos.y, grid.top())))),
                    None => {}
                }
            }
            let (Some(from), Some(to)) = (midnight(d), midnight(add_days(d, 1))) else { continue };
            let items: Vec<Appointment> = app.engine.session.appointments_between(from, to).into_iter().filter(|a| !a.all_day).collect();
            for a in &items {
                // While its bottom edge is dragged, the block shows the end it would get.
                let end = resizing.as_ref().filter(|r| r.id == a.id).map_or(a.end, |r| r.end);
                let top = ((a.start.max(from) - from) as f32 / 3600.0) * HOUR_HEIGHT;
                let bottom = ((end.min(to) - from) as f32 / 3600.0) * HOUR_HEIGHT;
                let block =
                    Rect::from_min_max(pos2(x + 2.0, grid.top() + top + 1.0), pos2(x + col_w - 2.0, grid.top() + bottom.max(top + 18.0) - 1.0));
                let label = format!("{} {}\n{}", hm(a.start), hm(end), chip_title(a));
                if chip(ui, block, &label, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), draggable(a), t) {
                    open = Some((a.id.clone(), a.start));
                }
                if draggable(a).is_none() || a.end > to {
                    continue;
                }
                // The bottom edge stretches or shortens the appointment, by the half hour.
                let handle = Rect::from_min_max(pos2(block.left(), block.bottom() - RESIZE_HANDLE), block.right_bottom());
                let h = ui.interact(handle, ui.make_persistent_id(("resize", &a.id)), Sense::drag());
                h.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Eindtijd van {} verslepen", a.title)));
                if h.hovered() || h.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
                if h.dragged()
                    && let Some(pos) = pointer
                    && let Some(end) = resized_end(a, d, grid_time(pos.y, grid.top()))
                {
                    resize = Some(Resizing { id: a.id.clone(), end });
                }
                if h.drag_stopped() {
                    resized = Some(a.id.clone());
                }
            }
            if d == today {
                let now = Local::now().naive_local().time();
                let y = grid.top() + (now.hour() as f32 + now.minute() as f32 / 60.0) * HOUR_HEIGHT;
                ui.painter().line_segment([pos2(x, y), pos2(x + col_w, y)], Stroke::new(2.0, t.flag));
            }
        }
    });
    if let Some(r) = resize {
        app.calendar.resizing = Some(r);
    }
    if let Some(id) = resized {
        app.finish_resize(&id);
    } else if let Some((id, day, time)) = task_drop {
        app.plan_task(&id, day, time);
    } else if let Some((id, day, time)) = drop {
        app.drop_appointment(&id, day, time);
    } else if let Some((id, start)) = open {
        app.open_appointment(&id, start);
    } else if let Some((d, hour)) = new_at {
        app.calendar.focus = d;
        app.new_appointment(d, hour);
    }
}

/// A small label that follows the pointer while an appointment or a task is dragged.
pub fn drag_preview(app: &MailApp, ctx: &egui::Context, t: &Tokens) {
    let text = if let Some(dragged) = egui::DragAndDrop::payload::<DraggedAppointment>(ctx) {
        format!("📅 {}", app.engine.session.appointment(&dragged.0).map(|a| a.title.clone()).unwrap_or_default())
    } else if let Some(task) = egui::DragAndDrop::payload::<DraggedTask>(ctx) {
        format!("✔ {}", app.engine.session.task(&task.0).map(|a| a.title.clone()).unwrap_or_default())
    } else {
        return;
    };
    let Some(pos) = ctx.pointer_interact_pos() else { return };
    egui::Area::new(egui::Id::new("appointment-drag-preview")).fixed_pos(pos + vec2(12.0, 12.0)).order(egui::Order::Tooltip).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.list).show(ui, |ui| {
            ui.label(RichText::new(text).color(t.text));
        });
    });
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
            let title = if app.calendar.is_searching() { "Zoekresultaten".to_string() } else { period_title(app.calendar.view, app.calendar.focus) };
            ui.label(RichText::new(title).size(18.0).strong().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if app.calendar.is_searching() && ui.small_button("✕").on_hover_text("Zoekopdracht wissen").clicked() {
                    app.calendar.query.clear();
                }
                let width = ui.available_width().clamp(80.0, 240.0);
                ui.add(
                    egui::TextEdit::singleline(&mut app.calendar.query)
                        .id(egui::Id::new(SEARCH_BOX))
                        .hint_text("Zoeken in agenda")
                        .desired_width(width),
                );
            });
        });
        ui.add_space(8.0);
        if app.calendar.is_searching() {
            search_results(app, ui, t);
            return;
        }
        match app.calendar.view {
            View::Month => month_view(app, ui, t),
            View::Week => time_grid(app, ui, t, 7),
            View::WorkWeek => time_grid(app, ui, t, 5),
            View::Day => time_grid(app, ui, t, 1),
        }
    });
}

/// The appointments that match the search box, from a year back to two years ahead, one row
/// per occurrence; a click opens it and moves the focus to its day.
fn search_results(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let today = today();
    let (Some(from), Some(to)) = (midnight(add_days(today, -SEARCH_BACK_DAYS)), midnight(add_days(today, SEARCH_AHEAD_DAYS))) else {
        return;
    };
    let list = app.engine.session.appointments_matching(app.calendar.query.trim(), from, to);
    let count = list.len();
    let summary = match count {
        0 => "Geen afspraak gevonden in het afgelopen jaar en de twee jaar hierna.".to_string(),
        1 => "1 afspraak gevonden.".to_string(),
        n => format!("{n} afspraken gevonden, van een jaar terug tot twee jaar vooruit."),
    };
    ui.label(RichText::new(summary).size(12.0).color(t.text_dim));
    ui.add_space(6.0);
    let selected = app.engine.session.selected_appointment.clone();
    let mut open: Option<(String, i64)> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for a in &list {
            let on = selected.as_deref() == Some(a.id.as_str());
            let r = ui.add_sized(
                [ui.available_width(), 44.0],
                egui::Button::new("").fill(if on { t.checked } else { Color32::TRANSPARENT }).frame_when_inactive(on),
            );
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, chip_title(a)));
            let rect = r.rect.shrink2(vec2(8.0, 4.0));
            let p = ui.painter();
            let dot = Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(12.0, 12.0));
            p.rect_filled(dot, CornerRadius::same(3), appointment_color(app, a, t));
            let x = rect.left() + 26.0;
            p.text(pos2(x, rect.top() + 2.0), Align2::LEFT_TOP, chip_title(a), FontId::proportional(13.5), t.text);
            let day = to_local(a.start).date();
            let when = if a.all_day {
                format!("{} {} {} {} · hele dag", weekday_name(day), day.day(), month_name(day.month()), day.year())
            } else {
                format!("{} {} {} {} · {} – {}", weekday_name(day), day.day(), month_name(day.month()), day.year(), hm(a.start), hm(a.end))
            };
            let sub = if a.location.is_empty() { when } else { format!("{when} · {}", a.location) };
            p.text(pos2(x, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, sub, FontId::proportional(11.5), t.text_dim);
            if r.clicked() {
                open = Some((a.id.clone(), a.start));
            }
        }
    });
    if let Some((id, start)) = open {
        app.calendar.focus = to_local(start).date();
        app.open_appointment(&id, start);
    }
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, width: f32) {
    ui.horizontal(|ui| {
        ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new(label).color(t.text)));
        ui.add(egui::TextEdit::singleline(value).desired_width(width));
    });
}

/// The "Herhalen" rows of the window: how often, which days, and when it stops.
fn repeat_fields(ui: &mut Ui, t: &Tokens, editor: &mut Editor) {
    let options: [(Option<Frequency>, &str); 5] = [
        (None, "Niet"),
        (Some(Frequency::Daily), "Elke dag"),
        (Some(Frequency::Weekly), "Elke week"),
        (Some(Frequency::Monthly), "Elke maand"),
        (Some(Frequency::Yearly), "Elk jaar"),
    ];
    let current = options.iter().find(|(f, _)| *f == editor.repeat).map(|(_, n)| *n).unwrap_or("Niet");
    ui.horizontal(|ui| {
        ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Herhalen").color(t.text)));
        egui::ComboBox::from_id_salt("appointment-repeat").selected_text(current).width(140.0).show_ui(ui, |ui| {
            for (f, n) in options {
                ui.selectable_value(&mut editor.repeat, f, n);
            }
        });
    });
    let Some(frequency) = editor.repeat else { return };
    let unit = match frequency {
        Frequency::Daily => "dagen",
        Frequency::Weekly => "weken",
        Frequency::Monthly => "maanden",
        Frequency::Yearly => "jaar",
    };
    ui.horizontal(|ui| {
        ui.add_space(94.0);
        ui.label(RichText::new("Om de").color(t.text));
        ui.add(egui::TextEdit::singleline(&mut editor.interval).desired_width(36.0).hint_text("1"));
        ui.label(RichText::new(unit).color(t.text));
    });
    if frequency == Frequency::Weekly {
        ui.horizontal(|ui| {
            ui.add_space(94.0);
            for (i, name) in WEEKDAYS_SHORT.iter().enumerate() {
                ui.checkbox(&mut editor.weekdays[i], *name);
            }
        });
    }
    ui.horizontal(|ui| {
        ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Eindigt").color(t.text)));
        ui.radio_value(&mut editor.ends, Ends::Never, "Nooit");
        ui.radio_value(&mut editor.ends, Ends::OnDate, "Op");
        ui.add_enabled(editor.ends == Ends::OnDate, egui::TextEdit::singleline(&mut editor.until).desired_width(90.0).hint_text("31-12-2026"));
        ui.radio_value(&mut editor.ends, Ends::AfterCount, "Na");
        ui.add_enabled(editor.ends == Ends::AfterCount, egui::TextEdit::singleline(&mut editor.count).desired_width(36.0));
        ui.label(RichText::new("keer").color(t.text));
    });
    // What the fields add up to, in words, as long as they parse.
    if let Ok(start) = NaiveDate::parse_from_str(editor.start_date.trim(), DATE_FORMAT)
        && let Ok(Some(r)) = editor.recurrence(start)
    {
        let mut text = describe_recurrence(&r);
        if let Some(u) = r.until {
            text.push_str(&format!(", tot en met {}", to_local(u).format(DATE_FORMAT)));
        }
        ui.horizontal(|ui| {
            ui.add_space(94.0);
            ui.label(RichText::new(text).size(12.0).color(t.text_dim));
        });
    }
}

/// The appointment window, when one is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let calendars: Vec<(String, String)> = app.engine.session.calendars.iter().map(|c| (c.id.clone(), c.name.clone())).collect();
    let local_calendars: Vec<String> = app.engine.session.calendars.iter().filter(|c| c.account_id.is_none()).map(|c| c.id.clone()).collect();
    let Some(editor) = app.calendar.editor.as_mut() else { return };
    let mut save = false;
    let mut delete = false;
    let mut skip = false;
    let mut forward = false;
    let mut close = false;
    let title = match (&editor.id, editor.repeat) {
        (None, _) => "Nieuwe afspraak",
        (Some(_), Some(_)) => "Herhaalde afspraak",
        (Some(_), None) => "Afspraak",
    };
    egui::Window::new(title).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
        field(ui, t, "Titel", &mut editor.title, 300.0);
        field(ui, t, "Locatie", &mut editor.location, 300.0);
        field(ui, t, crate::meeting::FIELD, &mut editor.attendees, 300.0);
        crate::contacts::complete_addresses(ui, t, &app.engine.session, &mut editor.attendees);
        if !editor.attendees.trim().is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(94.0);
                ui.label(
                    RichText::new("Verzenden slaat de afspraak op en mailt de genodigden een uitnodiging voor hun agenda.")
                        .size(12.0)
                        .color(t.text_dim),
                );
            });
        }
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
            ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Herinnering").color(t.text)));
            egui::ComboBox::from_id_salt("appointment-reminder").selected_text(reminder_label(editor.reminder)).width(160.0).show_ui(ui, |ui| {
                for (r, label) in REMINDERS {
                    ui.selectable_value(&mut editor.reminder, r, label);
                }
            });
        });
        // A calendar on this computer keeps a series as its rule. An account's server owns its
        // own series: a new appointment there can start one, but what comes back is listed and
        // edited one occurrence at a time.
        let chosen = editor.calendar_id.clone().or_else(|| calendars.first().map(|(id, _)| id.clone()));
        let local = chosen.as_ref().is_some_and(|id| local_calendars.contains(id));
        if local || editor.id.is_none() {
            repeat_fields(ui, t, editor);
            if !local && editor.repeat.is_some() {
                ui.label(
                    RichText::new("De reeks wordt in het account aangemaakt; daarna toont Mailcraft elke keer apart en bewerk je ze los van elkaar.")
                        .size(12.0)
                        .color(t.text_dim),
                );
            }
            if local && let Some(occurrence) = editor.occurrence {
                let when = to_local(occurrence);
                let text = format!(
                    "Geopend vanaf de keer op {} {} {}. Wijzigingen gelden voor de hele reeks.",
                    weekday_name(when.date()),
                    when.day(),
                    month_name(when.month())
                );
                ui.label(RichText::new(text).size(12.0).color(t.text_dim));
            }
        } else {
            editor.repeat = None;
        }
        ui.horizontal(|ui| {
            ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new("Notities").color(t.text)));
            ui.add(egui::TextEdit::multiline(&mut editor.notes).desired_width(300.0).desired_rows(4));
        });
        if let Some(e) = &editor.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = if editor.attendees.trim().is_empty() { "Opslaan" } else { "Verzenden" };
            save = ui.add(egui::Button::new(RichText::new(label).color(t.on_accent)).fill(t.accent).min_size(vec2(90.0, 26.0))).clicked();
            if editor.id.is_some() {
                if editor.occurrence.is_some() {
                    skip = ui
                        .add(egui::Button::new("Deze keer overslaan").min_size(vec2(90.0, 26.0)))
                        .on_hover_text("Alleen deze keer vervalt; de reeks blijft.")
                        .clicked();
                }
                let label = if editor.occurrence.is_some() { "Reeks verwijderen" } else { "Verwijderen" };
                delete = ui.add(egui::Button::new(label).min_size(vec2(90.0, 26.0))).clicked();
                forward = ui
                    .add(egui::Button::new(crate::meeting::FORWARD).min_size(vec2(90.0, 26.0)))
                    .on_hover_text("Opent een nieuw bericht met de afspraak als bijlage, zodat iemand anders hem in zijn agenda kan zetten.")
                    .clicked();
            }
            close = ui.add(egui::Button::new("Annuleren").min_size(vec2(90.0, 26.0))).clicked();
        });
    });
    if skip && let (Some(id), Some(start)) = (editor.id.clone(), editor.occurrence) {
        match app.engine.run("appointment.skip", &json!({ "id": id, "start": start })) {
            Ok(()) => close = true,
            Err(e) => editor.error = Some(describe(&e)),
        }
    }
    if save {
        match editor.params().and_then(|p| crate::meeting::attendees(&editor.attendees).map(|a| (p, a))) {
            Ok((mut params, attendees)) => {
                // An invitation goes out from the calendar's account; without a real one the
                // appointment is not saved either, so nothing half happens.
                let calendar = editor.calendar_id.clone().or_else(|| calendars.first().map(|(id, _)| id.clone()));
                let account = if attendees.is_empty() { None } else { crate::meeting::account_for(&app.engine.session, calendar.as_deref()) };
                if !attendees.is_empty() && account.is_none() {
                    editor.error = Some(crate::meeting::NO_ACCOUNT.to_string());
                } else {
                    if !attendees.is_empty() {
                        params["notes"] = json!(crate::meeting::noted(&editor.notes, &attendees));
                    }
                    match app.engine.run("appointment.save", &params) {
                        Ok(()) => {
                            if let Some(account) = account
                                && let Some(a) = app.engine.session.selected_appointment().cloned()
                            {
                                let draft = crate::meeting::draft(&a, &account.address, attendees);
                                app.engine.send(&account.id, draft);
                            }
                            close = true;
                        }
                        Err(e) => editor.error = Some(describe(&e)),
                    }
                }
            }
            Err(e) => editor.error = Some(e),
        }
    }
    if forward && let Some(id) = editor.id.clone() {
        match app.engine.session.appointment(&id).cloned() {
            Some(a) => {
                app.compose = Some(crate::meeting::forward_window(&app.engine.session, &a));
                close = true;
            }
            None => editor.error = Some(describe(&CommandError::NotFound)),
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
        CommandError::Param("id") => "Een herhalende afspraak versleep je niet; open de reeks om de tijd te veranderen.".into(),
        CommandError::Param("end") => "Het einde ligt voor het begin.".into(),
        CommandError::Param("recurrence") => {
            "De herhaling kan zo niet: een afspraak moet korter zijn dan de tijd tussen twee keren, en herhalen kan alleen in een agenda op deze computer.".into()
        }
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
    fn a_drop_lands_on_the_day_at_the_right_time() {
        let base = to_timestamp(d(2026, 10, 21).and_hms_opt(14, 0, 0).unwrap()).unwrap();
        let a = Appointment {
            id: "a".into(),
            calendar_id: "local".into(),
            title: "Tandarts".into(),
            start: base,
            end: base + 3_600,
            all_day: false,
            location: String::new(),
            notes: String::new(),
            recurrence: None,
            reminder: None,
        };
        let moved = to_local(dropped_start(&a, d(2026, 10, 23), None).unwrap());
        assert_eq!((moved.date(), moved.time()), (d(2026, 10, 23), NaiveTime::from_hms_opt(14, 0, 0).unwrap()), "a month drop keeps the time");
        let moved = to_local(dropped_start(&a, d(2026, 10, 23), Some(NaiveTime::from_hms_opt(9, 30, 0).unwrap())).unwrap());
        assert_eq!(moved.time(), NaiveTime::from_hms_opt(9, 30, 0).unwrap(), "a grid drop takes the half hour");
        let all_day = Appointment { all_day: true, ..a.clone() };
        let moved = to_local(dropped_start(&all_day, d(2026, 10, 23), Some(NaiveTime::from_hms_opt(9, 30, 0).unwrap())).unwrap());
        assert_eq!((moved.date(), moved.time()), (d(2026, 10, 23), NaiveTime::MIN), "an all-day appointment stays all day");
        assert!(draggable(&a).is_some());
        assert!(draggable(&Appointment { recurrence: Some(Recurrence::every(Frequency::Weekly)), ..a.clone() }).is_none(), "a series is not dragged");

        assert_eq!(grid_time(100.0, 100.0), NaiveTime::MIN);
        assert_eq!(grid_time(100.0 + HOUR_HEIGHT * 9.6, 100.0), NaiveTime::from_hms_opt(9, 30, 0).unwrap(), "snapped to the half hour");
        assert_eq!(grid_time(100.0 + HOUR_HEIGHT * 30.0, 100.0), NaiveTime::from_hms_opt(23, 30, 0).unwrap(), "clamped to the day");
        assert_eq!(grid_time(0.0, 100.0), NaiveTime::MIN);

        let at = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert_eq!(resized_end(&a, d(2026, 10, 21), at(15, 30)), Some(base + 2 * 3_600), "the end of the half hour under the pointer");
        let task = Task { id: "t".into(), title: "Offerte".into(), notes: "voor Ravi".into(), ..Task::default() };
        let e = planned(&task, d(2026, 10, 23), Some(at(9, 30)));
        assert_eq!((e.title.as_str(), e.notes.as_str(), e.all_day), ("Offerte", "voor Ravi", false));
        assert_eq!((e.start_date.as_str(), e.start_time.as_str(), e.end_time.as_str()), ("23-10-2026", "09:30", "10:30"), "an hour at the drop");
        let e = planned(&task, d(2026, 10, 23), None);
        assert!(e.all_day && e.start_date == "23-10-2026" && e.id.is_none(), "a day drop plans the whole day");
        assert_eq!(resized_end(&a, d(2026, 10, 21), at(14, 0)), Some(base + 1_800), "shortened to the first half hour");
        assert_eq!(resized_end(&a, d(2026, 10, 21), at(9, 0)), Some(base + 1_800), "never before half an hour after the start");
    }

    #[test]
    fn titles_and_steps_follow_the_view() {
        let focus = d(2026, 10, 21);
        assert_eq!(period_title(View::Month, focus), "oktober 2026");
        assert_eq!(period_title(View::Week, focus), "19 – 25 oktober 2026");
        assert_eq!(period_title(View::Week, d(2026, 10, 30)), "26 oktober – 1 november 2026");
        assert_eq!(period_title(View::WorkWeek, focus), "19 – 23 oktober 2026");
        assert_eq!(period_title(View::WorkWeek, d(2026, 11, 1)), "26 – 30 oktober 2026", "a Sunday belongs to the work week before it");
        assert_eq!(step(View::WorkWeek, focus, 1), d(2026, 10, 28));
        assert_eq!(period_title(View::Day, focus), "woensdag 21 oktober 2026");
        assert_eq!(period(View::Day, focus), (focus, focus));
        assert_eq!(period(View::WorkWeek, focus), (d(2026, 10, 19), d(2026, 10, 23)));
        assert_eq!(period(View::Week, d(2026, 10, 30)), (d(2026, 10, 26), d(2026, 11, 1)));
        assert_eq!(period(View::Month, focus), (d(2026, 10, 1), d(2026, 10, 31)));
        assert_eq!(period(View::Month, d(2026, 2, 10)), (d(2026, 2, 1), d(2026, 2, 28)));
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
            recurrence: None,
            reminder: None,
        };
        let back = Editor::from_appointment(&a, None);
        assert_eq!((back.start_date.as_str(), back.end_date.as_str()), ("21-10-2026", "21-10-2026"));
        assert!(back.repeat.is_none() && back.occurrence.is_none() && back.reminder.is_none());
        assert_eq!(p["reminder"], 15, "a new appointment reminds a quarter of an hour before");
        assert!(back.params().unwrap()["reminder"].is_null());
        assert_eq!(reminder_label(Some(1440)), "1 dag ervoor");
        assert_eq!(reminder_label(Some(45)), "45 minuten ervoor");
    }

    #[test]
    fn repeat_fields_become_a_recurrence_and_back() {
        let mut e = Editor::new_at(d(2026, 10, 21).and_hms_opt(14, 0, 0).unwrap());
        e.title = "Pianoles".into();
        assert!(e.params().unwrap()["recurrence"].is_null());
        e.repeat = Some(Frequency::Weekly);
        e.weekdays[4] = true;
        e.ends = Ends::OnDate;
        e.until = "31-12-2026".into();
        let p = e.params().unwrap();
        let r: Recurrence = serde_json::from_value(p["recurrence"].clone()).unwrap();
        assert_eq!((r.frequency, r.interval, r.weekdays.as_slice(), r.count), (Frequency::Weekly, 1, &[4][..], None));
        assert_eq!(to_local(r.until.unwrap()).date(), d(2026, 12, 31), "until is the end of that day");
        assert!(p["recurrence"].get("skipped").is_none(), "the window never sends the skipped list");
        e.weekdays = [false; 7];
        let r: Recurrence = serde_json::from_value(e.params().unwrap()["recurrence"].clone()).unwrap();
        assert_eq!(r.weekdays, vec![2], "no day ticked means the start's day");
        e.ends = Ends::AfterCount;
        e.count = "0".into();
        assert!(e.params().unwrap_err().contains("hoeveel keer"));
        e.count = "5".into();
        e.interval = "1000".into();
        assert!(e.params().unwrap_err().contains("1 tot 999"));
        e.interval = "2".into();
        e.until = "01-01-2020".into();
        e.ends = Ends::OnDate;
        assert_eq!(e.params().unwrap_err(), "De herhaling eindigt voor de eerste keer.");

        let a = Appointment {
            id: "x".into(),
            calendar_id: "local".into(),
            title: "Pianoles".into(),
            start: to_timestamp(d(2026, 10, 21).and_hms_opt(14, 0, 0).unwrap()).unwrap(),
            end: to_timestamp(d(2026, 10, 21).and_hms_opt(15, 0, 0).unwrap()).unwrap(),
            all_day: false,
            location: String::new(),
            notes: String::new(),
            recurrence: Some(Recurrence { interval: 2, count: Some(8), ..Recurrence::every(Frequency::Monthly) }),
            reminder: None,
        };
        let back = Editor::from_appointment(&a, Some(a.start + 86_400 * 61));
        assert_eq!((back.repeat, back.interval.as_str(), back.ends, back.count.as_str()), (Some(Frequency::Monthly), "2", Ends::AfterCount, "8"));
        assert_eq!(back.occurrence, Some(a.start + 86_400 * 61));
        let p = back.params().unwrap();
        assert_eq!(p["recurrence"]["count"], 8);
        assert_eq!(p["id"], "x");
    }
}
