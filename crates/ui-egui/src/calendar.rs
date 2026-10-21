//! The calendar module: month, week and day views over the session's appointments, and the
//! appointment window. Times are shown in the computer's time zone. A recurring appointment
//! shows every occurrence; opening one opens the series, with a button to skip just that time.
//! A search box above the view lists the matching appointments of the past year and the two
//! years ahead instead of the view, until it is cleared. A block can be dragged to another day
//! (month view) or half hour (week and day views); a series cannot, it is changed in its window.

use chrono::{DateTime, Datelike, Days, Local, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use egui::{Align2, Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use merel_engine::{CommandError, tasks::TaskFilter};
use merel_model::{Appointment, Attendee, Frequency, Meeting, Recurrence, ShowAs, Task, describe_recurrence};

use crate::helder::{self, Icon};
use crate::{MailApp, Module, Tokens};

pub const MONTHS: [&str; 12] =
    ["januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november", "december"];
pub const WEEKDAYS: [&str; 7] = ["maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag", "zondag"];
const WEEKDAYS_SHORT: [&str; 7] = ["ma", "di", "wo", "do", "vr", "za", "zo"];

/// The width of the week-number column in the month view, when it is shown.
const WEEK_GUTTER: f32 = 30.0;

/// The label of the week-number cell left of a week, as Outlook's "Weeknummers weergeven".
pub(crate) fn week_label(d: NaiveDate) -> String {
    format!("Week {}", d.iso_week().week())
}

/// A cell with the week number of the week `d` falls in, named so a screen reader (and the
/// tests) find it.
fn week_cell(ui: &mut Ui, t: &Tokens, size: egui::Vec2, d: NaiveDate) {
    let (rect, r) = ui.allocate_exact_size(size, Sense::hover());
    let label = week_label(d);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone()));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, d.iso_week().week().to_string(), FontId::proportional(11.0), t.text_dim);
}
/// Marks an occurrence of a recurring appointment in the views.
const REPEAT_MARK: &str = "🔁";
/// Outlook's "Privé": the lock before a private appointment, and the checkbox in the window.
const PRIVATE_MARK: &str = "🔒";
pub const PRIVATE: &str = "Privé";
/// The search box's widget id, so Ctrl+E can put the cursor there.
pub(crate) const SEARCH_BOX: &str = "calendar_search_box";
/// How far a search looks: this many days back and ahead of today.
const SEARCH_BACK_DAYS: i64 = 365;
const SEARCH_AHEAD_DAYS: i64 = 731;

/// Pixels per hour in the week and day views.
const HOUR_HEIGHT: f32 = 44.0;
/// The height of the day headers above the hour grid: weekday and day number.
const DAY_HEADER: f32 = 50.0;
/// The hour the hour grid opens at; earlier hours are a scroll up.
const FIRST_HOUR: f32 = 7.0;

/// The checkbox on the Beeld tab that shades the hours outside the working day.
pub const WORK_HOURS: &str = "Werkuren";
/// The working day the shading leaves clear: from 8:00 up to 17:00, Monday to Friday, as
/// Outlook's default.
pub const WORK_START: u32 = 8;
pub const WORK_END: u32 = 17;

/// Whether the hour starting at `hour` on `d` falls outside the working day.
pub fn outside_work(d: NaiveDate, hour: u32) -> bool {
    d.weekday().num_days_from_monday() >= 5 || !(WORK_START..WORK_END).contains(&hour)
}
const DATE_FORMAT: &str = "%d-%m-%Y";

/// The ribbon button and window that jump to a typed day, Outlook's "Ga naar datum" (Ctrl+G).
pub const GOTO: &str = "Ga naar datum";
pub const GOTO_HINT: &str = "Gebruik dag-maand-jaar, bijvoorbeeld 21-10-2026.";
const GOTO_BOX: &str = "calendar_goto_box";
const GOTO_PICKER: &str = "Dag kiezen";
/// The calendar buttons beside the appointment's dates.
pub const START_PICKER: &str = "Begindatum kiezen";
pub const END_PICKER: &str = "Einddatum kiezen";
pub const UNTIL_PICKER: &str = "Laatste datum van de reeks kiezen";

/// A day typed as dag-maand-jaar, with or without leading zeros.
pub fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), DATE_FORMAT).ok()
}

impl MailApp {
    /// Opens "Ga naar datum" with the shown day filled in.
    pub fn open_goto(&mut self) {
        self.calendar.goto = Some(self.calendar.focus.format(DATE_FORMAT).to_string());
    }
}

/// The "Ga naar datum" window, while it is open: a day as dag-maand-jaar, "Ga" (or Enter) shows
/// it in the current view, "Annuleren" closes.
pub fn goto_window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut text) = app.calendar.goto.take() else { return };
    let mut go = false;
    let mut close = false;
    if helder::popup(ctx, t, "goto-date", GOTO, 260.0, |ui| {
        ui.label(RichText::new("Welke dag wil je zien? (dag-maand-jaar)").color(t.text));
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0).id(egui::Id::new(GOTO_BOX)));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            // Picking a day from the month goes there at once.
            if crate::date_picker::button(ui, t, &mut text, GOTO_PICKER).is_some() {
                go = true;
            }
        });
        if parse_date(&text).is_none() {
            ui.label(RichText::new(GOTO_HINT).size(12.0).color(t.flag));
        }
        ui.horizontal(|ui| {
            if ui.add(helder::primary_text(t, "Ga")).clicked() {
                go = true;
            }
            if ui.button("Annuleren").clicked() {
                close = true;
            }
        });
    }) {
        close = true;
    }
    if go && let Some(day) = parse_date(&text) {
        app.calendar.focus = day;
        app.calendar.query.clear();
        close = true;
    }
    if !close {
        app.calendar.goto = Some(text);
    }
}
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
    /// The day typed in the "Ga naar datum" window, while it is open.
    pub goto: Option<String>,
    /// The window to add, rename or delete a calendar, while it is open.
    pub dialog: Option<crate::calendars::Dialog>,
}

/// An appointment being stretched or shortened by its bottom edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resizing {
    pub id: String,
    pub end: i64,
}

impl Default for State {
    fn default() -> Self {
        State { view: View::Month, focus: today(), query: String::new(), editor: None, resizing: None, goto: None, dialog: None }
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
pub(crate) fn week_start(d: NaiveDate) -> NaiveDate {
    d.checked_sub_days(Days::new(u64::from(d.weekday().num_days_from_monday()))).unwrap_or(d)
}

pub(crate) fn add_days(d: NaiveDate, n: i64) -> NaiveDate {
    let moved = if n >= 0 { d.checked_add_days(Days::new(n.unsigned_abs())) } else { d.checked_sub_days(Days::new(n.unsigned_abs())) };
    moved.unwrap_or(d)
}

pub(crate) fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
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
    /// The addresses to invite, as typed; a finished address (after `;`, `,` or Enter) becomes a
    /// chip in `invited`. "Verzenden" then saves the appointment and mails everyone a meeting
    /// request from the calendar's account. Empty for an appointment of your own.
    pub attendees: String,
    /// Outlook's "Optioneel": the addresses typed on that line.
    pub optional: String,
    /// Whether the Optioneel line shows.
    pub show_optional: bool,
    /// The people invited, as chips with their answers.
    pub invited: Vec<Attendee>,
    /// The meeting as it was saved, when the appointment is one.
    pub meeting: Option<Meeting>,
    /// The attendees came from the notes line of an older version; the meeting is made on save.
    pub legacy: bool,
    pub all_day: bool,
    pub start_date: String,
    pub start_time: String,
    pub end_date: String,
    pub end_time: String,
    pub notes: String,
    /// The formatting over `notes`, with the editor's undo and selection.
    pub notes_editor: crate::rich_edit::Editor,
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
    /// Outlook's "Privé".
    pub private: bool,
    /// Outlook's "Weergeven als".
    pub show_as: ShowAs,
    /// The category names on the appointment.
    pub categories: Vec<String>,
    pub error: Option<String>,
}

/// The "Weergeven als" choices of the window, in Outlook's order.
const SHOW_AS: [(ShowAs, &str); 4] = [(ShowAs::Free, "Vrij"), (ShowAs::Tentative, "Voorlopig"), (ShowAs::Busy, "Bezet"), (ShowAs::Oof, "Afwezig")];
/// Outlook paints an out-of-office appointment purple, whatever its calendar's colour.
const OOF_COLOR: Color32 = Color32::from_rgb(122, 86, 170);

pub fn show_as_label(show_as: ShowAs) -> &'static str {
    SHOW_AS.iter().find(|(s, _)| *s == show_as).map(|(_, label)| *label).unwrap_or("Bezet")
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
            optional: String::new(),
            show_optional: false,
            invited: Vec::new(),
            meeting: None,
            legacy: false,
            all_day: false,
            start_date: start.format(DATE_FORMAT).to_string(),
            start_time: start.format(TIME_FORMAT).to_string(),
            end_date: end.format(DATE_FORMAT).to_string(),
            end_time: end.format(TIME_FORMAT).to_string(),
            notes: String::new(),
            notes_editor: crate::rich_edit::Editor::new("appointment-notes", "", None),
            repeat: None,
            interval: "1".into(),
            weekdays: [false; 7],
            ends: Ends::Never,
            until: String::new(),
            count: "10".into(),
            occurrence: None,
            // Outlook's default: a quarter of an hour before.
            reminder: Some(15),
            private: false,
            show_as: merel_model::ShowAs::Busy,
            categories: Vec::new(),
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
        e.private = a.private;
        e.show_as = a.show_as;
        e.categories = a.categories.clone();
        e.end_date = end.format(DATE_FORMAT).to_string();
        e.end_time = end.format(TIME_FORMAT).to_string();
        e.notes = a.notes.clone();
        e.meeting = a.meeting.clone();
        e.invited = a.meeting.as_ref().map(|m| m.attendees.clone()).unwrap_or_default();
        // An older version kept the people on a line in the notes; they come back as chips.
        if a.meeting.is_none()
            && let Some((people, notes)) = merel_engine::meetings::legacy_attendees(&a.notes)
        {
            e.invited = people;
            e.notes = notes;
            e.legacy = true;
        }
        e.show_optional = e.invited.iter().any(|p| p.optional);
        e.notes_editor = crate::rich_edit::Editor::new("appointment-notes", &e.notes, a.notes_rich.clone());
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
            .filter(|n| (1..=merel_model::MAX_INTERVAL).contains(n))
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
            "notes_rich": self.notes_editor.stored(&self.notes),
            "recurrence": recurrence,
            "reminder": self.reminder,
            "private": self.private,
            "show_as": self.show_as,
            "categories": self.categories,
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
            self.notice = Some(crate::Notice::warning("Die tijd bestaat niet in deze tijdzone."));
            return;
        };
        if start == a.start {
            return;
        }
        self.notice = self.engine.run("appointment.move", &json!({ "id": id, "start": start })).err().map(|e| notice(&e));
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
        self.notice = self.engine.run("appointment.resize", &json!({ "id": id, "end": r.end })).err().map(|e| notice(&e));
    }

    /// Selects an appointment and opens it in the window; `start` is the occurrence that was
    /// clicked, which matters for a recurring one.
    pub fn open_appointment(&mut self, id: &str, start: i64) {
        self.run("appointment.select", json!({ "id": id }));
        if let Some(a) = self.engine.session.appointment(id) {
            self.calendar.editor = Some(Editor::from_appointment(a, Some(start)));
        }
    }

    /// Does what a click on an appointment's block, or its right-click menu, asked for.
    pub(crate) fn chip_action(&mut self, id: &str, start: i64, action: ChipAction) {
        match action {
            ChipAction::Open => self.open_appointment(id, start),
            ChipAction::Skip => self.run("appointment.skip", json!({ "id": id, "start": start })),
            ChipAction::Delete => {
                self.run("appointment.select", json!({ "id": id }));
                self.run("appointment.delete", serde_json::Value::Null);
            }
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
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(Margin { left: 4, right: 12, top: 4, bottom: 10 }))
        .show(ui, |ui| {
            let focus = app.calendar.focus;
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} {}", month_name(focus.month()), focus.year())).size(14.0).strong().color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    helder::quiet(ui, t, |ui| {
                        if nav_button(ui, t, Icon::ChevronRight, "Volgende maand").clicked() {
                            app.calendar.focus = add_months(focus, 1);
                        }
                        if nav_button(ui, t, Icon::ChevronLeft, "Vorige maand").clicked() {
                            app.calendar.focus = add_months(focus, -1);
                        }
                    });
                });
            });
            ui.add_space(4.0);
            let (shown_from, shown_to) = period(app.calendar.view, focus);
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
                        // The days the big view shows, softly marked, as the design has it.
                        if app.calendar.view != View::Month && day >= shown_from && day <= shown_to && day != today {
                            p.circle_filled(rect.center(), cell / 2.0 - 2.0, t.checked);
                        }
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
                            t.text_disabled
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
            ui.add_space(12.0);
            crate::calendars::list(app, ui, t);
            task_list(app, ui, t);
        });
}

/// How many open tasks the calendar's side pane lists.
const SIDE_TASKS: usize = 12;

/// Hover text of a calendar's tick box in the side pane.
pub const CALENDAR_TICK_HINT: &str = "Vinkje weg verbergt de afspraken van deze agenda in de weergaven; herinneringen komen wel.";

/// The occurrences of calendars whose tick box is on, in the order given.
pub(crate) fn shown(app: &MailApp, list: Vec<Appointment>) -> Vec<Appointment> {
    list.into_iter().filter(|a| app.view.calendar_shown(&a.calendar_id)).collect()
}

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
        // Each task a small card with an open circle, ready to be dragged into the calendar.
        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click_and_drag());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &task.title));
        r.dnd_set_drag_payload(DraggedTask(task.id.clone()));
        let p = ui.painter();
        p.rect(
            rect,
            CornerRadius::same(helder::CONTROL_RADIUS),
            if r.hovered() { t.hover } else { t.card },
            Stroke::new(1.0, t.card_border),
            egui::StrokeKind::Inside,
        );
        p.circle_stroke(pos2(rect.left() + 18.0, rect.center().y), 7.0, Stroke::new(1.5, t.text_dim));
        let text_rect = Rect::from_min_max(pos2(rect.left() + 34.0, rect.top()), rect.right_bottom() - vec2(8.0, 0.0));
        let galley = p.layout_no_wrap(task.title.clone(), FontId::proportional(13.0), t.text);
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

/// The colour of an appointment's block: its first category's, as Outlook paints a categorised
/// appointment, else its calendar's.
#[cfg(test)]
pub(crate) fn block_color(app: &MailApp, a: &Appointment, t: &Tokens) -> Color32 {
    appointment_color(app, a, t)
}

fn appointment_color(app: &MailApp, a: &Appointment, t: &Tokens) -> Color32 {
    if let Some(name) = a.categories.first() {
        return crate::categories::color_of(app.engine.session.category_color(name));
    }
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
    let mut s = String::new();
    if a.private {
        s += PRIVATE_MARK;
        s.push(' ');
    }
    if a.recurrence.is_some() {
        s += REPEAT_MARK;
        s.push(' ');
    }
    s + &a.title
}

/// How an appointment block is painted.
struct ChipPaint {
    fill: Color32,
    border: Option<Stroke>,
    text: Color32,
    /// The calendar's colour as a bar down the left edge.
    stripe: Option<Color32>,
}

/// The block's look for a free/busy status, in the Helder style: a soft tint of the calendar's
/// colour with dark text and a bar of the full colour at the left. A free appointment is the
/// card's own white with the colour as a border, a tentative one a lighter tint with that
/// border, a busy one the tint with the bar, an out-of-office one the same in purple.
fn chip_paint(color: Color32, show_as: ShowAs, hovered: bool, t: &Tokens) -> ChipPaint {
    let strength = match (t.dark, hovered) {
        (false, false) => 0.18,
        (false, true) => 0.26,
        (true, false) => 0.22,
        (true, true) => 0.28,
    };
    let tint = |c: Color32, f: f32| crate::theme::over(c, t.card, f);
    match show_as {
        ShowAs::Free => {
            ChipPaint { fill: if hovered { tint(color, 0.08) } else { t.card }, border: Some(Stroke::new(1.5, color)), text: t.text, stripe: None }
        }
        ShowAs::Tentative => {
            ChipPaint { fill: tint(color, strength / 2.0), border: Some(Stroke::new(1.0, color)), text: t.text, stripe: Some(color) }
        }
        ShowAs::Busy => ChipPaint { fill: tint(color, strength), border: None, text: t.text, stripe: Some(color) },
        ShowAs::Oof => ChipPaint { fill: tint(OOF_COLOR, strength), border: None, text: t.text, stripe: Some(OOF_COLOR) },
    }
}

/// The width of the coloured bar at the left of a block.
const STRIPE: f32 = 3.0;

/// What a click on an appointment's block, or a choice in its right-click menu, asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChipAction {
    Open,
    /// This occurrence of a series only.
    Skip,
    Delete,
}

/// The right-click menu of an appointment, as Outlook's.
pub(crate) const OPEN_MENU: &str = "Openen";
pub(crate) const DELETE_ONE_MENU: &str = "Dit exemplaar verwijderen";
pub(crate) const DELETE_SERIES_MENU: &str = "Hele reeks verwijderen";
pub(crate) const CANCEL_MENU: &str = "Vergadering annuleren…";

/// A clickable block for one appointment, a drag source when `drag` is given, with a menu on
/// a right click. Returns what a click or the menu asked for.
#[allow(clippy::too_many_arguments)]
fn chip(
    ui: &mut Ui,
    rect: Rect,
    label: &str,
    a: &Appointment,
    fill: Color32,
    selected: bool,
    drag: Option<DraggedAppointment>,
    t: &Tokens,
) -> Option<ChipAction> {
    let show_as = a.show_as;
    let sense = if drag.is_some() { Sense::click_and_drag() } else { Sense::click() };
    let r = ui.interact(rect, ui.make_persistent_id(("chip", rect.min.x as i32, rect.min.y as i32, label)), sense);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if let Some(payload) = drag {
        r.dnd_set_drag_payload(payload);
    }
    let p = ui.painter();
    let look = chip_paint(fill, show_as, r.hovered(), t);
    paint_block(p, rect, &look);
    if selected {
        p.rect_stroke(rect, CornerRadius::same(6), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    let text_rect = rect.shrink2(vec2(if look.stripe.is_some() { 8.0 } else { 6.0 }, 2.0));
    let galley = p.layout(label.to_string(), FontId::proportional(12.0), look.text, text_rect.width());
    p.with_clip_rect(text_rect).galley(text_rect.left_top(), galley, look.text);
    let mut action = r.clicked().then_some(ChipAction::Open);
    // A meeting with people is called off from its window, where the cancellation is sent.
    let invited = a.meeting.as_ref().is_some_and(|m| !m.attendees.is_empty());
    r.context_menu(|ui| {
        if ui.button(OPEN_MENU).clicked() {
            action = Some(ChipAction::Open);
        }
        ui.separator();
        if invited {
            if ui.button(CANCEL_MENU).clicked() {
                action = Some(ChipAction::Open);
            }
        } else if a.recurrence.is_some() {
            if ui.button(DELETE_ONE_MENU).clicked() {
                action = Some(ChipAction::Skip);
            }
            if ui.button(DELETE_SERIES_MENU).clicked() {
                action = Some(ChipAction::Delete);
            }
        } else if ui.button("Verwijderen").clicked() {
            action = Some(ChipAction::Delete);
        }
    });
    action
}

/// A quiet square button with a chevron, for the previous or next day, week or month.
pub(crate) fn nav_button(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str) -> egui::Response {
    let r = ui.add(
        egui::Button::image(icon.image(helder::ICON, t.text))
            .frame_when_inactive(false)
            .corner_radius(helder::CONTROL_RADIUS)
            .min_size(vec2(32.0, 32.0)),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    r.on_hover_text(label)
}

/// The rounded block with its fill, border and bar.
fn paint_block(p: &egui::Painter, rect: Rect, look: &ChipPaint) {
    p.rect_filled(rect, CornerRadius::same(6), look.fill);
    if let Some(stripe) = look.stripe {
        let bar = Rect::from_min_size(rect.min, vec2(STRIPE, rect.height()));
        p.rect_filled(bar, CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, stripe);
    }
    if let Some(border) = look.border {
        p.rect_stroke(rect, CornerRadius::same(6), border, egui::StrokeKind::Inside);
    }
}

fn month_view(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let focus = app.calendar.focus;
    let first = focus.with_day(1).unwrap_or(focus);
    let start = week_start(first);
    let today = today();
    let selected = app.engine.session.selected_appointment.clone();
    let week_numbers = app.view.week_numbers;
    let gutter = if week_numbers { WEEK_GUTTER } else { 0.0 };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let w = (ui.available_width() - gutter) / 7.0;
        if week_numbers {
            ui.allocate_exact_size(vec2(gutter, 22.0), Sense::hover());
        }
        for d in WEEKDAYS_SHORT {
            let (rect, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, d, FontId::proportional(12.0), t.text_dim);
        }
    });
    let avail = ui.available_size();
    let w = (avail.x - gutter) / 7.0;
    let h = (avail.y / 6.0).max(60.0);
    let mut open: Option<(String, i64, ChipAction)> = None;
    let mut pick_day: Option<NaiveDate> = None;
    let mut drop: Option<(String, NaiveDate)> = None;
    let mut task_drop: Option<(String, NaiveDate)> = None;
    let mut day = start;
    for _ in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if week_numbers {
                week_cell(ui, t, vec2(gutter, h), day);
            }
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
                p.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
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
                    let items = shown(app, app.engine.session.appointments_between(from, to));
                    let room = ((rect.height() - 26.0) / 20.0).floor().max(0.0) as usize;
                    let shown = if items.len() > room { room.saturating_sub(1) } else { items.len() };
                    for (i, a) in items.iter().take(shown).enumerate() {
                        let y = rect.top() + 24.0 + i as f32 * 20.0;
                        let chip_rect = Rect::from_min_size(pos2(rect.left() + 3.0, y), vec2(rect.width() - 6.0, 18.0));
                        let label = if a.all_day { chip_title(a) } else { format!("{} {}", hm(a.start), chip_title(a)) };
                        if let Some(action) =
                            chip(ui, chip_rect, &label, a, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), draggable(a), t)
                        {
                            open = Some((a.id.clone(), a.start, action));
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
    } else if let Some((id, start, action)) = open {
        app.chip_action(&id, start, action);
    } else if let Some(d) = pick_day {
        app.calendar.focus = d;
        app.calendar.view = View::Day;
    }
}

/// The week and day views: an all-day strip, then the hours.
/// The calendars whose tick box is on, in the side pane's order.
pub(crate) fn shown_calendars(app: &MailApp) -> Vec<(String, String, Color32)> {
    app.engine
        .session
        .calendars
        .iter()
        .filter(|c| app.view.calendar_shown(&c.id))
        .map(|c| (c.id.clone(), app.engine.session.calendar_label(c), crate::calendars::rgb(c.color)))
        .collect()
}

/// The panes of the day and week views: one merged pane (`None`), or with "Naast elkaar
/// weergeven" and more than one calendar shown, one per calendar.
pub(crate) fn panes(app: &MailApp) -> Vec<Option<String>> {
    let shown = shown_calendars(app);
    if app.view.calendars_side_by_side && shown.len() > 1 { shown.into_iter().map(|(id, _, _)| Some(id)).collect() } else { vec![None] }
}

/// The label of a calendar tab's button that puts the calendars side by side.
pub fn split_label(name: &str) -> String {
    format!("{name} naast elkaar weergeven")
}

/// The label of a calendar tab's button that merges the calendars into one grid again.
pub fn merge_label(name: &str) -> String {
    format!("{name} samenvoegen")
}

/// The label of a calendar tab's close button, which unticks the calendar.
pub fn close_label(name: &str) -> String {
    format!("{name} sluiten")
}

/// The height of the row of calendar tabs.
const TAB_HEIGHT: f32 = 28.0;

/// What a click on a calendar tab asks for.
enum TabAction {
    Toggle,
    Close(String),
}

/// One calendar tab, Outlook's: the name on a tint of the calendar's colour with a bar in the
/// full colour under it, an arrow that puts the calendars side by side or merges them, and a
/// cross that unticks the calendar.
fn calendar_tab(ui: &mut Ui, t: &Tokens, rect: Rect, id: &str, name: &str, color: Color32, split: bool) -> Option<TabAction> {
    let p = ui.painter();
    let tab = rect.shrink2(vec2(2.0, 0.0));
    p.rect_filled(tab, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, crate::theme::over(color, t.card, 0.25));
    p.rect_filled(Rect::from_min_max(pos2(tab.left(), tab.bottom() - 3.0), tab.right_bottom()), CornerRadius::ZERO, color);
    let button = 22.0;
    let mut action = None;
    let arrow = Rect::from_min_size(pos2(tab.left() + 3.0, tab.center().y - button / 2.0 - 1.5), vec2(button, button));
    let close = Rect::from_min_size(pos2(tab.right() - button - 3.0, arrow.top()), vec2(button, button));
    let (icon, label) = if split { (Icon::ArrowLeft, merge_label(name)) } else { (Icon::ArrowRight, split_label(name)) };
    for (r, icon, label, what) in [(arrow, icon, label, TabAction::Toggle), (close, Icon::X, close_label(name), TabAction::Close(id.to_string()))] {
        let resp = ui.interact(r, ui.make_persistent_id(("tab", id, &label)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if resp.hovered() || resp.has_focus() {
            ui.painter().rect_filled(r, CornerRadius::same(4), crate::theme::over(color, t.card, 0.45));
        }
        icon.image(14.0, t.text).paint_at(ui, Rect::from_center_size(r.center(), vec2(14.0, 14.0)));
        if resp.on_hover_text(&label).clicked() {
            action = Some(what);
        }
    }
    let text = Rect::from_min_max(pos2(arrow.right() + 4.0, tab.top()), pos2(close.left() - 2.0, tab.bottom() - 3.0));
    let p = ui.painter();
    let galley = p.layout_no_wrap(name.to_string(), FontId::proportional(13.0), t.text);
    p.with_clip_rect(text).galley(pos2(text.left(), text.center().y - galley.size().y / 2.0), galley, t.text);
    action
}

/// The lane of each appointment in a day column and how many lanes its group of overlapping
/// appointments needs, so appointments at the same time stand side by side, as Outlook puts
/// them. `items` are in order of start.
fn lanes(items: &[(i64, i64)]) -> Vec<(usize, usize)> {
    let mut out = vec![(0, 1); items.len()];
    // The group being laid out: where it starts in `items`, when its last appointment ends,
    // and when each of its lanes is free again.
    let mut group = 0;
    let mut group_end = i64::MIN;
    let mut free_at: Vec<i64> = Vec::new();
    for (i, &(start, end)) in items.iter().enumerate() {
        if start >= group_end {
            for slot in &mut out[group..i] {
                slot.1 = free_at.len();
            }
            group = i;
            free_at.clear();
        }
        let lane = free_at.iter().position(|e| *e <= start).unwrap_or_else(|| {
            free_at.push(end);
            free_at.len() - 1
        });
        free_at[lane] = end;
        out[i].0 = lane;
        group_end = group_end.max(end);
    }
    for slot in &mut out[group..] {
        slot.1 = free_at.len().max(1);
    }
    out
}

fn time_grid(app: &mut MailApp, ui: &mut Ui, t: &Tokens, days: i64) {
    let first = if days > 1 { week_start(app.calendar.focus) } else { app.calendar.focus };
    let today = today();
    let selected = app.engine.session.selected_appointment.clone();
    let gutter = 48.0;
    let mut open: Option<(String, i64, ChipAction)> = None;
    let mut new_at: Option<(NaiveDate, u32, Option<String>)> = None;
    let mut drop: Option<(String, NaiveDate, Option<NaiveTime>)> = None;
    let mut task_drop: Option<(String, NaiveDate, Option<NaiveTime>)> = None;
    let mut resize: Option<Resizing> = None;
    let mut resized: Option<String> = None;
    let mut tab_action = None;
    let resizing = app.calendar.resizing.clone();
    let calendars = shown_calendars(app);
    let panes = panes(app);
    let split = panes.len() > 1;
    let pane_w = ((ui.available_width() - gutter) / panes.len() as f32).floor();
    let col_w = (pane_w / days as f32).max(if split { 20.0 } else { 40.0 });
    let pane_w = col_w * days as f32;
    let in_pane = |a: &Appointment, pane: &Option<String>| pane.as_ref().is_none_or(|id| *id == a.calendar_id);
    // With more than one calendar ticked, a tab for each: over its own days when side by side,
    // in a row over the merged grid otherwise.
    if calendars.len() > 1 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (row, _) = ui.allocate_exact_size(vec2(gutter + pane_w * panes.len() as f32, TAB_HEIGHT), Sense::hover());
            if split {
                for (i, (id, name, color)) in calendars.iter().enumerate() {
                    let left = row.left() + gutter + i as f32 * pane_w;
                    let rect = Rect::from_min_size(pos2(left, row.top()), vec2(pane_w.min(220.0), TAB_HEIGHT));
                    if let Some(a) = calendar_tab(ui, t, rect, id, name, *color, true) {
                        tab_action = Some(a);
                    }
                }
            } else {
                let width = ((pane_w) / calendars.len() as f32).min(200.0);
                for (i, (id, name, color)) in calendars.iter().enumerate() {
                    let rect = Rect::from_min_size(pos2(row.left() + gutter + i as f32 * width, row.top()), vec2(width, TAB_HEIGHT));
                    if let Some(a) = calendar_tab(ui, t, rect, id, name, *color, false) {
                        tab_action = Some(a);
                    }
                }
            }
        });
    }
    // Day headers, with the week number in the time gutter when asked for.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if app.view.week_numbers {
            week_cell(ui, t, vec2(gutter, DAY_HEADER), first);
        } else {
            ui.allocate_exact_size(vec2(gutter, DAY_HEADER), Sense::hover());
        }
        for _ in &panes {
            for i in 0..days {
                let d = add_days(first, i);
                let (rect, _) = ui.allocate_exact_size(vec2(col_w, DAY_HEADER), Sense::hover());
                let p = ui.painter();
                // The weekday small and grey over the day's number, today tinted in the app colour.
                if d == today {
                    p.rect_filled(rect.shrink(2.0), CornerRadius::same(helder::CONTROL_RADIUS), t.checked);
                }
                let (name_color, day_color) = if d == today { (t.accent_text, t.accent_text) } else { (t.text_dim, t.text) };
                let left = rect.left() + if split && days > 1 { 4.0 } else { 10.0 };
                p.text(pos2(left, rect.top() + 6.0), Align2::LEFT_TOP, weekday_name(d), FontId::proportional(12.0), name_color);
                p.text(pos2(left, rect.top() + 21.0), Align2::LEFT_TOP, d.day().to_string(), FontId::proportional(20.0), day_color);
            }
        }
    });
    // All-day strip.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let strip_h = 26.0;
        let (g, _) = ui.allocate_exact_size(vec2(gutter, strip_h), Sense::hover());
        ui.painter().text(g.right_center() - vec2(4.0, 0.0), Align2::RIGHT_CENTER, "hele dag", FontId::proportional(10.5), t.text_dim);
        for pane in &panes {
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
                ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
                if let (Some(from), Some(to)) = (midnight(d), midnight(add_days(d, 1))) {
                    let items: Vec<Appointment> =
                        shown(app, app.engine.session.appointments_between(from, to)).into_iter().filter(|a| a.all_day && in_pane(a, pane)).collect();
                    if let Some(a) = items.first() {
                        let label = if items.len() > 1 { format!("{} (+{})", chip_title(a), items.len() - 1) } else { chip_title(a) };
                        let chip_rect = rect.shrink2(vec2(2.0, 3.0));
                        if let Some(action) =
                            chip(ui, chip_rect, &label, a, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), draggable(a), t)
                        {
                            open = Some((a.id.clone(), a.start, action));
                        }
                    }
                }
            }
        }
    });
    let mut hours = egui::ScrollArea::vertical().auto_shrink(false).id_salt("hours");
    if egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id("hours")).is_none() {
        // The first time, the working day is in view rather than the night.
        hours = hours.vertical_scroll_offset(FIRST_HOUR * HOUR_HEIGHT);
    }
    hours.show(ui, |ui| {
        let (grid, _) = ui.allocate_exact_size(vec2(gutter + pane_w * panes.len() as f32, HOUR_HEIGHT * 24.0), Sense::hover());
        let p = ui.painter();
        let columns: Vec<(usize, i64, f32)> =
            (0..panes.len()).flat_map(|n| (0..days).map(move |i| (n, i, grid.left() + gutter + n as f32 * pane_w + i as f32 * col_w))).collect();
        if app.view.work_hours {
            // The hours outside the working day in a quieter shade, as Outlook draws them.
            let shade = if t.dark { Color32::from_white_alpha(7) } else { Color32::from_black_alpha(11) };
            for &(_, i, x) in &columns {
                let d = add_days(first, i);
                for hour in (0..24).filter(|h| outside_work(d, *h)) {
                    let y = grid.top() + hour as f32 * HOUR_HEIGHT;
                    p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(col_w, HOUR_HEIGHT)), CornerRadius::ZERO, shade);
                }
            }
        }
        for hour in 0..24 {
            let y = grid.top() + hour as f32 * HOUR_HEIGHT;
            p.line_segment([pos2(grid.left() + gutter, y), pos2(grid.right(), y)], Stroke::new(1.0, t.card_border));
            p.text(pos2(grid.left() + gutter - 6.0, y + 2.0), Align2::RIGHT_TOP, format!("{hour:02}:00"), FontId::proportional(11.0), t.text_dim);
        }
        for &(n, i, x) in &columns {
            let pane = &panes[n];
            let d = add_days(first, i);
            let column = Rect::from_min_size(pos2(x, grid.top()), vec2(col_w, grid.height()));
            // A heavier line where one calendar's days end and the next one's begin.
            let edge = if split && i == 0 && n > 0 { Stroke::new(2.0, t.control_border) } else { Stroke::new(1.0, t.card_border) };
            ui.painter().line_segment([column.left_top(), column.left_bottom()], edge);
            let r = ui.interact(column, ui.make_persistent_id(("column", n, i)), Sense::click());
            r.widget_info(|| {
                let day = format!("{} {} {}", weekday_name(d), d.day(), month_name(d.month()));
                let name = pane.as_ref().and_then(|id| calendars.iter().find(|(c, _, _)| c == id)).map(|(_, name, _)| name.as_str());
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.map_or(day.clone(), |name| format!("{day}, {name}")))
            });
            if r.clicked()
                && let Some(pos) = r.interact_pointer_pos()
            {
                let hour = ((pos.y - grid.top()) / HOUR_HEIGHT).floor().clamp(0.0, 23.0) as u32;
                new_at = Some((d, hour, pane.clone()));
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
            let items: Vec<Appointment> =
                shown(app, app.engine.session.appointments_between(from, to)).into_iter().filter(|a| !a.all_day && in_pane(a, pane)).collect();
            let ends: Vec<i64> = items.iter().map(|a| resizing.as_ref().filter(|r| r.id == a.id).map_or(a.end, |r| r.end)).collect();
            let spans: Vec<(i64, i64)> = items.iter().zip(&ends).map(|(a, end)| (a.start, (*end).max(a.start + 1))).collect();
            let lanes = lanes(&spans);
            for ((a, &end), &(lane, count)) in items.iter().zip(&ends).zip(&lanes) {
                // While its bottom edge is dragged, the block shows the end it would get.
                let top = ((a.start.max(from) - from) as f32 / 3600.0) * HOUR_HEIGHT;
                let bottom = ((end.min(to) - from) as f32 / 3600.0) * HOUR_HEIGHT;
                let lane_w = (col_w - 4.0) / count as f32;
                let left = x + 2.0 + lane as f32 * lane_w;
                let block = Rect::from_min_max(
                    pos2(left, grid.top() + top + 1.0),
                    pos2(left + lane_w - if lane + 1 < count { 2.0 } else { 0.0 }, grid.top() + bottom.max(top + 18.0) - 1.0),
                );
                // A short block keeps the title on one line with the start, a longer one puts the
                // times under the title.
                let label = if block.height() < 34.0 {
                    format!("{} {}", chip_title(a), hm(a.start))
                } else {
                    format!("{}\n{} – {}", chip_title(a), hm(a.start), hm(end))
                };
                if let Some(action) =
                    chip(ui, block, &label, a, appointment_color(app, a, t), selected.as_deref() == Some(a.id.as_str()), draggable(a), t)
                {
                    open = Some((a.id.clone(), a.start, action));
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
    match tab_action {
        Some(TabAction::Toggle) => {
            app.view.calendars_side_by_side = !app.view.calendars_side_by_side;
            let ctx = ui.ctx().clone();
            app.save_view(&ctx);
        }
        Some(TabAction::Close(id)) => {
            app.view.set_calendar_shown(&id, false);
            let ctx = ui.ctx().clone();
            app.save_view(&ctx);
        }
        None => {}
    }
    if let Some(r) = resize {
        app.calendar.resizing = Some(r);
    }
    if let Some(id) = resized {
        app.finish_resize(&id);
    } else if let Some((id, day, time)) = task_drop {
        app.plan_task(&id, day, time);
    } else if let Some((id, day, time)) = drop {
        app.drop_appointment(&id, day, time);
    } else if let Some((id, start, action)) = open {
        app.chip_action(&id, start, action);
    } else if let Some((d, hour, pane)) = new_at {
        app.calendar.focus = d;
        app.new_appointment(d, hour);
        // A click in a calendar's own days makes the appointment in that calendar.
        if let (Some(id), Some(editor)) = (pane, app.calendar.editor.as_mut()) {
            editor.calendar_id = Some(id);
        }
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
    // The calendar itself is a white card on the window's ground.
    let frame = helder::card_frame(t).inner_margin(Margin::same(12)).outer_margin(Margin { left: 0, right: 12, top: 0, bottom: 12 });
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            helder::quiet(ui, t, |ui| {
                if nav_button(ui, t, Icon::ChevronLeft, "Vorige").clicked() {
                    app.calendar.focus = step(app.calendar.view, app.calendar.focus, -1);
                }
                if nav_button(ui, t, Icon::ChevronRight, "Volgende").clicked() {
                    app.calendar.focus = step(app.calendar.view, app.calendar.focus, 1);
                }
            });
            ui.add_space(8.0);
            let title = if app.calendar.is_searching() { "Zoekresultaten".to_string() } else { period_title(app.calendar.view, app.calendar.focus) };
            ui.label(RichText::new(title).size(helder::TITLE_TEXT).strong().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if app.calendar.is_searching()
                    && crate::helder::quiet(ui, t, |ui| crate::helder::icon_button(ui, t, crate::helder::Icon::X, crate::SEARCH_CLEAR)).clicked()
                {
                    app.calendar.query.clear();
                }
                let width = ui.available_width().clamp(80.0, 280.0);
                helder::search_field(ui, t, &mut app.calendar.query, egui::Id::new(SEARCH_BOX), "Zoeken in agenda", width);
            });
        });
        ui.add_space(10.0);
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
    let list = shown(app, app.engine.session.appointments_matching(app.calendar.query.trim(), from, to));
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
            let sub = if a.show_as == ShowAs::Busy { sub } else { format!("{sub} · {}", show_as_label(a.show_as)) };
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

/// The "Herhalen" choices of the window, as Outlook names them.
const REPEATS: [(Option<Frequency>, &str); 5] = [
    (None, "Niet herhalen"),
    (Some(Frequency::Daily), "Elke dag"),
    (Some(Frequency::Weekly), "Elke week"),
    (Some(Frequency::Monthly), "Elke maand"),
    (Some(Frequency::Yearly), "Elk jaar"),
];

/// The "Herhalen" row of the window and, when it repeats, how often, which days, and when it
/// stops.
fn repeat_fields(ui: &mut Ui, t: &Tokens, editor: &mut Editor) {
    let current = REPEATS.iter().find(|(f, _)| *f == editor.repeat).map(|(_, n)| *n).unwrap_or("Niet herhalen");
    helder::icon_row(ui, t, Icon::Repeat, |ui| {
        helder::select(ui, t, current, 200.0, |ui| {
            for (f, n) in REPEATS {
                if ui.selectable_label(editor.repeat == f, n).clicked() {
                    editor.repeat = f;
                    ui.close();
                }
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
    ui.add_space(2.0);
    row(ui, |ui| {
        helder::indent(ui);
        ui.label(RichText::new("Om de").size(14.0).color(t.text));
        helder::number_field(ui, t, &mut editor.interval, "1");
        ui.label(RichText::new(unit).size(14.0).color(t.text));
    });
    if frequency == Frequency::Weekly {
        ui.add_space(4.0);
        row(ui, |ui| {
            helder::indent(ui);
            ui.spacing_mut().item_spacing.x = 6.0;
            for (i, name) in WEEKDAYS_SHORT.iter().enumerate() {
                if helder::day_toggle(ui, t, name, editor.weekdays[i]).clicked() {
                    editor.weekdays[i] = !editor.weekdays[i];
                }
            }
        });
    }
    ui.add_space(4.0);
    row(ui, |ui| {
        helder::indent(ui);
        ui.label(RichText::new("Eindigt").size(14.0).color(t.text));
        ui.add_space(4.0);
        for (ends, name) in [(Ends::Never, "Nooit"), (Ends::OnDate, "Op een datum"), (Ends::AfterCount, "Na een aantal keer")] {
            if helder::chip(ui, t, name, editor.ends == ends).clicked() {
                editor.ends = ends;
            }
        }
    });
    match editor.ends {
        Ends::Never => {}
        Ends::OnDate => {
            row(ui, |ui| {
                helder::indent(ui);
                ui.label(RichText::new("Tot en met").size(14.0).color(t.text));
                crate::date_picker::field(ui, t, &mut editor.until, "31-12-2026", 110.0, UNTIL_PICKER);
            });
        }
        Ends::AfterCount => {
            row(ui, |ui| {
                helder::indent(ui);
                ui.label(RichText::new("Na").size(14.0).color(t.text));
                helder::number_field(ui, t, &mut editor.count, "10");
                ui.label(RichText::new("keer").size(14.0).color(t.text));
            });
        }
    }
    // What the fields add up to, in words, as long as they parse.
    if let Ok(start) = NaiveDate::parse_from_str(editor.start_date.trim(), DATE_FORMAT)
        && let Ok(Some(r)) = editor.recurrence(start)
    {
        let mut text = describe_recurrence(&r);
        if let Some(u) = r.until {
            text.push_str(&format!(", tot en met {}", to_local(u).format(DATE_FORMAT)));
        }
        helder::note(ui, t, &text);
    }
}

/// A row whose label and fields share one centre line.
fn row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::left_to_right(egui::Align::Center), add).inner
}

/// The times a time field's list offers: every half hour.
fn half_hours() -> impl Iterator<Item = String> {
    (0..48).map(|i| format!("{:02}:{:02}", i / 2, (i % 2) * 30))
}

/// A time field with a chevron that lists the half hours; returns the time picked from the
/// list, if one was.
fn time_field(ui: &mut Ui, t: &Tokens, value: &mut String, id: &str) -> Option<String> {
    helder::underline_field(ui, t, value, "09:00", 48.0, false);
    let mut picked = None;
    let button =
        egui::Button::image(Icon::ChevronDown.image(16.0, t.text_dim)).frame_when_inactive(false).min_size(vec2(24.0, helder::BUTTON_HEIGHT));
    egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        ui.push_id(id, |ui| {
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for time in half_hours() {
                    if ui.selectable_label(*value == time, &time).clicked() {
                        picked = Some(time);
                        ui.close();
                    }
                }
            });
        });
    });
    picked
}

impl Editor {
    /// The start as the fields have it, when they parse.
    fn start(&self) -> Option<NaiveDateTime> {
        let date = NaiveDate::parse_from_str(self.start_date.trim(), DATE_FORMAT).ok()?;
        if self.all_day {
            return Some(date.and_time(NaiveTime::MIN));
        }
        Some(date.and_time(NaiveTime::parse_from_str(self.start_time.trim(), TIME_FORMAT).ok()?))
    }

    /// The end as the fields have it, when they parse.
    fn end(&self) -> Option<NaiveDateTime> {
        let date = NaiveDate::parse_from_str(self.end_date.trim(), DATE_FORMAT).ok()?;
        if self.all_day {
            return date.checked_add_days(Days::new(1)).map(|d| d.and_time(NaiveTime::MIN));
        }
        Some(date.and_time(NaiveTime::parse_from_str(self.end_time.trim(), TIME_FORMAT).ok()?))
    }

    /// Moves the start to `time` and the end along with it, so the appointment keeps its
    /// length, as Outlook does when the start changes.
    pub fn move_start(&mut self, time: &str) {
        let before = self.start().zip(self.end()).map(|(s, e)| e - s);
        self.start_time = time.to_string();
        if let (Some(start), Some(length)) = (self.start(), before)
            && length > chrono::TimeDelta::zero()
            && let Some(end) = start.checked_add_signed(length)
        {
            self.end_date = end.format(DATE_FORMAT).to_string();
            self.end_time = end.format(TIME_FORMAT).to_string();
        }
    }

    /// Everyone the window invites: the chips, then what is still typed on both lines; or what
    /// is wrong with the typed addresses.
    pub fn all_attendees(&self) -> Result<Vec<Attendee>, String> {
        let mut all = self.invited.clone();
        for (text, optional) in [(&self.attendees, false), (&self.optional, true)] {
            for a in crate::meeting::attendees(text)? {
                if !all.iter().any(|p| p.email.eq_ignore_ascii_case(a.email())) {
                    all.push(crate::meeting::attendee(&a, optional));
                }
            }
        }
        Ok(all)
    }

    /// Whether anyone is invited or being typed.
    pub fn has_attendees(&self) -> bool {
        !self.invited.is_empty() || !self.attendees.trim().is_empty() || !self.optional.trim().is_empty()
    }

    /// Turns the finished addresses typed on a line (those before the last `;` or `,`, or all of
    /// them when `whole`) into chips; text that is not an address yet stays typed.
    pub fn chip_typed(&mut self, optional: bool, whole: bool) {
        let text = if optional { &mut self.optional } else { &mut self.attendees };
        let cut = if whole { Some(text.len()) } else { text.rfind([';', ',']).map(|i| i + 1) };
        let Some(cut) = cut else { return };
        let Ok(list) = crate::meeting::attendees(&text[..cut]) else { return };
        let rest = text[cut..].trim_start().to_string();
        *text = rest;
        for a in list {
            if !self.invited.iter().any(|p| p.email.eq_ignore_ascii_case(a.email())) {
                self.invited.push(crate::meeting::attendee(&a, optional));
            }
        }
    }

    /// Whether the window differs from `stored` in what the attendees must hear about: the
    /// title, the time, the place, the repetition or who is invited.
    pub fn changed_for_attendees(&self, stored: &Appointment) -> bool {
        let Ok(p) = self.params() else { return true };
        let before: Vec<String> = match (&stored.meeting, merel_engine::meetings::legacy_attendees(&stored.notes)) {
            (Some(m), _) => m.attendees.iter().map(|a| a.email.to_lowercase()).collect(),
            (None, Some((people, _))) => people.iter().map(|a| a.email.to_lowercase()).collect(),
            (None, None) => Vec::new(),
        };
        let mut now: Vec<String> = self.all_attendees().unwrap_or_default().iter().map(|a| a.email.to_lowercase()).collect();
        let mut before = before;
        now.sort();
        before.sort();
        p["title"].as_str().map(str::trim) != Some(stored.title.trim())
            || p["location"].as_str().map(str::trim) != Some(stored.location.trim())
            || p["start"].as_i64() != Some(stored.start)
            || p["end"].as_i64() != Some(stored.end)
            || p["all_day"].as_bool() != Some(stored.all_day)
            || serde_json::from_value::<Option<Recurrence>>(p["recurrence"].clone()).ok().flatten().map(|r| r.normalized())
                != stored.recurrence.clone().map(|r| r.normalized())
            || now != before
    }

    /// Puts `start` and `end` in the time fields, as "Voorgestelde tijd overnemen" does.
    pub fn set_times(&mut self, start: i64, end: i64) {
        let (s, e) = (to_local(start), to_local(end));
        self.start_date = s.format(DATE_FORMAT).to_string();
        self.start_time = s.format(TIME_FORMAT).to_string();
        self.end_date = e.format(DATE_FORMAT).to_string();
        self.end_time = e.format(TIME_FORMAT).to_string();
    }
}

/// What the main button of the appointment window does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainAction {
    Save,
    /// Saves and mails the invitations.
    Send,
    /// Saves and mails the change to the attendees.
    Update,
}

/// The line "Deze gebeurtenis vindt plaats in het verleden." over an appointment that is over.
pub const IN_THE_PAST: &str = "Deze gebeurtenis vindt plaats in het verleden.";
/// The button by a proposed new time that puts it in the fields.
pub const TAKE_PROPOSAL: &str = "Voorgestelde tijd overnemen";

/// The label of the chip button that takes `a` off the meeting.
pub fn remove_attendee_label(a: &Attendee) -> String {
    format!("{} verwijderen", a.label())
}

/// One invited person as a chip: the answer's mark, the name, and ✕ to take them off when the
/// person organises the meeting. Returns whether ✕ was clicked.
fn attendee_chip(ui: &mut Ui, t: &Tokens, a: &Attendee, removable: bool) -> bool {
    let mut off = false;
    let hover = format!("{} · {}", a.email, crate::meeting::response_label(a.response));
    egui::Frame::new().fill(t.checked).corner_radius(14).inner_margin(Margin { left: 10, right: 6, top: 3, bottom: 3 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mark = crate::meeting::response_mark(a.response);
        if !mark.is_empty() {
            let color = match a.response {
                merel_model::Response::Accepted => t.success,
                merel_model::Response::Declined => t.flag,
                _ => t.text_dim,
            };
            ui.label(RichText::new(mark).size(13.0).strong().color(color)).on_hover_text(crate::meeting::response_label(a.response));
        }
        ui.label(RichText::new(a.label()).size(13.0).color(t.text)).on_hover_text(&hover);
        if removable {
            let r = ui.add(egui::Button::new(RichText::new("✕").size(11.0).color(t.text_dim)).frame_when_inactive(false).min_size(vec2(18.0, 18.0)));
            let label = remove_attendee_label(a);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            off = r.on_hover_text("Haalt deze genodigde van de vergadering").clicked();
        } else {
            ui.add_space(4.0);
        }
    });
    off
}

/// One line of attendees, as Outlook's: the chips, then the field where more are typed.
fn attendee_line(ui: &mut Ui, t: &Tokens, editor: &mut Editor, optional: bool, organiser: bool, wide: f32) -> Option<String> {
    let mut off = None;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(wide);
        for a in editor.invited.iter().filter(|a| a.optional == optional) {
            if attendee_chip(ui, t, a, organiser) {
                off = Some(a.email.clone());
            }
        }
        if organiser {
            let hint = match (optional, editor.invited.iter().any(|a| a.optional == optional)) {
                (true, _) => "Optionele genodigden toevoegen",
                (false, false) => "Genodigden toevoegen",
                (false, true) => "",
            };
            let room = (ui.available_width() - if optional { 0.0 } else { 90.0 }).max(160.0);
            let text = if optional { &mut editor.optional } else { &mut editor.attendees };
            let r = helder::underline_field(ui, t, text, hint, room, false);
            let r = r.on_hover_text(if optional { crate::meeting::OPTIONAL } else { crate::meeting::FIELD });
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if r.changed() || entered {
                editor.chip_typed(optional, entered);
                if entered {
                    r.request_focus();
                }
            }
        }
        if !optional && organiser && !editor.show_optional {
            helder::right(ui, |ui| {
                if helder::add_link(ui, t, crate::meeting::OPTIONAL).on_hover_text("Nodigt iemand uit die er niet bij hoeft te zijn").clicked() {
                    editor.show_optional = true;
                }
            });
        }
    });
    off
}

/// An appointment as the day preview of the window draws it.
struct Busy {
    title: String,
    start: NaiveDateTime,
    end: NaiveDateTime,
    all_day: bool,
    color: Color32,
    show_as: ShowAs,
}

/// The hours the day preview shows.
const PREVIEW_HOURS: u32 = 9;
const PREVIEW_HOUR_HEIGHT: f32 = 40.0;

/// What else is in the shown calendars on the day the window's appointment starts.
fn day_of(app: &MailApp, editor: &Editor, t: &Tokens) -> Option<(NaiveDate, Vec<Busy>)> {
    let day = NaiveDate::parse_from_str(editor.start_date.trim(), DATE_FORMAT).ok()?;
    let from = midnight(day)?;
    let to = midnight(add_days(day, 1))?;
    let list = shown(app, app.engine.session.appointments_between(from, to));
    let busy = list
        .iter()
        .filter(|a| Some(&a.id) != editor.id.as_ref())
        .map(|a| Busy {
            title: chip_title(a),
            start: to_local(a.start),
            end: to_local(a.end),
            all_day: a.all_day,
            color: appointment_color(app, a, t),
            show_as: a.show_as,
        })
        .collect();
    Some((day, busy))
}

/// The day of the appointment, beside the form: the hours around it with what is already
/// planned, and the appointment itself in the app colour, so a clash shows before saving.
fn day_preview(ui: &mut Ui, t: &Tokens, editor: &Editor, day: NaiveDate, busy: &[Busy]) {
    ui.label(RichText::new(format!("{} {} {}", weekday_name(day), day.day(), month_name(day.month()))).size(15.0).strong().color(t.text));
    let span = editor.start().zip(editor.end());
    let clash: Vec<&str> = match span {
        Some((s, e)) => {
            busy.iter().filter(|b| !b.all_day && b.show_as != ShowAs::Free && b.start < e && s < b.end).map(|b| b.title.as_str()).collect()
        }
        None => Vec::new(),
    };
    let status = if clash.is_empty() { "Je hebt dan nog niets gepland.".to_string() } else { format!("Overlapt met {}.", clash.join(", ")) };
    ui.label(RichText::new(status).size(12.0).color(if clash.is_empty() { t.text_dim } else { t.flag }));
    for b in busy.iter().filter(|b| b.all_day) {
        ui.label(helder::dotted(&b.title, b.color, t));
    }
    ui.add_space(8.0);
    let first = span.map(|(s, _)| s.hour().saturating_sub(2)).unwrap_or(7).min(24 - PREVIEW_HOURS);
    let size = vec2(ui.available_width(), PREVIEW_HOURS as f32 * PREVIEW_HOUR_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect.expand(2.0));
    let gutter = 44.0;
    let y_of = |dt: NaiveDateTime| {
        let minutes = if dt.date() < day {
            0.0
        } else if dt.date() > day {
            24.0 * 60.0
        } else {
            (dt.hour() * 60 + dt.minute()) as f32
        };
        rect.top() + (minutes / 60.0 - first as f32) * PREVIEW_HOUR_HEIGHT
    };
    for i in 0..PREVIEW_HOURS {
        let y = rect.top() + i as f32 * PREVIEW_HOUR_HEIGHT;
        p.hline(rect.left() + gutter..=rect.right(), y, Stroke::new(1.0, t.card_border));
        p.text(pos2(rect.left() + gutter - 8.0, y), Align2::RIGHT_TOP, format!("{}:00", first + i), FontId::proportional(12.0), t.text_dim);
    }
    // On a clash the planned blocks take the left half and the new one the right half, so
    // both stay readable.
    let middle = rect.left() + gutter + (rect.width() - gutter) / 2.0;
    let (planned_right, new_left) = if clash.is_empty() { (rect.right() - 2.0, rect.left() + gutter + 4.0) } else { (middle - 2.0, middle + 2.0) };
    let block = |top: f32, bottom: f32, left: f32, right: f32| {
        let top = top.max(rect.top());
        let bottom = bottom.min(rect.bottom());
        (bottom > top + 2.0).then(|| Rect::from_min_max(pos2(left, top + 1.0), pos2(right, bottom - 1.0)))
    };
    let label = |r: Rect, text: &str, color: Color32| {
        let galley = p.layout(text.to_string(), FontId::proportional(12.0), color, r.width() - 12.0);
        p.with_clip_rect(r.intersect(rect)).galley(r.min + vec2(6.0, 4.0), galley, color);
    };
    for b in busy.iter().filter(|b| !b.all_day) {
        if let Some(r) = block(y_of(b.start), y_of(b.end), rect.left() + gutter + 4.0, planned_right) {
            let look = chip_paint(b.color, b.show_as, false, t);
            paint_block(&p, r, &look);
            label(r, &b.title, look.text);
        }
    }
    if !editor.all_day
        && let Some((s, e)) = span
        && let Some(r) = block(y_of(s), y_of(e), new_left, rect.right() - 2.0)
    {
        p.rect(r, CornerRadius::same(6), t.accent, Stroke::new(2.0, t.card), egui::StrokeKind::Outside);
        let title = if editor.title.trim().is_empty() { "Nieuwe afspraak" } else { editor.title.trim() };
        label(r, &format!("{title}\n{} – {}", s.format(TIME_FORMAT), e.format(TIME_FORMAT)), t.on_accent);
    }
}

/// The colour of a "Weergeven als" choice, as the dot before it shows it.
fn show_as_color(show_as: ShowAs, t: &Tokens) -> Color32 {
    match show_as {
        ShowAs::Free => t.control_border,
        ShowAs::Tentative => crate::theme::over(t.accent, t.card, 0.5),
        ShowAs::Busy => t.accent,
        ShowAs::Oof => OOF_COLOR,
    }
}

/// The menu of an opened time of a series that removes this time or the whole series, and
/// its two choices.
pub const DELETE_MENU: &str = "Verwijderen…";
pub const SKIP_ONE: &str = "Deze keer overslaan";
pub const DELETE_SERIES: &str = "Hele reeks verwijderen";

/// The appointment window, when one is open: a command bar on top, the form in a card, and
/// the day it falls on beside it.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let calendars: Vec<(String, String, Color32)> = app
        .engine
        .session
        .calendars
        .iter()
        .map(|c| (c.id.clone(), app.engine.session.calendar_label(c), Color32::from_rgb(c.color[0], c.color[1], c.color[2])))
        .collect();
    let local_calendars: Vec<String> = app.engine.session.calendars.iter().filter(|c| c.account_id.is_none()).map(|c| c.id.clone()).collect();
    let preview = app.calendar.editor.as_ref().and_then(|e| day_of(app, e, t));
    let stored = app.calendar.editor.as_ref().and_then(|e| e.id.as_ref()).and_then(|id| app.engine.session.appointment(id)).cloned();
    let Some(editor) = app.calendar.editor.as_mut() else { return };
    let organiser = editor.meeting.as_ref().is_none_or(|m| crate::meeting::organised_here(&app.engine.session, m));
    // As Outlook: an appointment with people sends, a meeting that changed sends the update.
    let action = if !organiser || !editor.has_attendees() && editor.meeting.is_none() {
        MainAction::Save
    } else if editor.meeting.is_none() && !editor.legacy {
        MainAction::Send
    } else if stored.as_ref().is_none_or(|s| editor.changed_for_attendees(s)) {
        MainAction::Update
    } else {
        MainAction::Save
    };
    let calls_off = organiser && editor.meeting.as_ref().is_some_and(|m| !m.attendees.is_empty()) && editor.occurrence.is_none();
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
    let mut open = true;
    helder::item_window(ctx, t, "appointment-window", title, vec2(980.0, 760.0), &mut open, |ui| {
        helder::quiet(ui, t, |ui| {
            ui.horizontal(|ui| {
                let main = match action {
                    MainAction::Send => helder::primary_button(ui, t, Icon::Send, "Verzenden")
                        .on_hover_text("Slaat de afspraak op en mailt de genodigden een uitnodiging voor hun agenda."),
                    MainAction::Update => helder::primary_button(ui, t, Icon::Send, crate::meeting::SEND_UPDATE)
                        .on_hover_text("Slaat de wijziging op en mailt hem de genodigden; wie eraf is gehaald, krijgt een annulering."),
                    MainAction::Save => helder::primary_button(ui, t, Icon::Save, "Opslaan"),
                };
                save = main.clicked();
                helder::bar_separator(ui, t);
                helder::menu_button(ui, t, None, helder::dotted(show_as_label(editor.show_as), show_as_color(editor.show_as, t), t), |ui| {
                    for (s, label) in SHOW_AS {
                        if ui.add(egui::Button::new(helder::dotted(label, show_as_color(s, t), t)).selected(editor.show_as == s)).clicked() {
                            editor.show_as = s;
                            ui.close();
                        }
                    }
                })
                .on_hover_text("Weergeven als");
                helder::menu_button(ui, t, Some(Icon::Bell), RichText::new(reminder_label(editor.reminder)).size(14.0).color(t.text), |ui| {
                    for (r, label) in REMINDERS {
                        if ui.selectable_label(editor.reminder == r, label).clicked() {
                            editor.reminder = r;
                            ui.close();
                        }
                    }
                })
                .on_hover_text("Herinnering");
                if helder::toggle_button(ui, t, Icon::Lock, PRIVATE, editor.private)
                    .on_hover_text("Wie je agenda deelt, ziet alleen dat je bezet bent.")
                    .clicked()
                {
                    editor.private = !editor.private;
                }
                if editor.id.is_some() {
                    forward = helder::tool_button(ui, t, Icon::Forward, crate::meeting::FORWARD)
                        .on_hover_text("Opent een nieuw bericht met de afspraak als bijlage, zodat iemand anders hem in zijn agenda kan zetten.")
                        .clicked();
                }
                // The right-hand group keeps its own width, so the bar never draws it over the
                // buttons before it; the window grows instead.
                helder::trailing(ui, |ui| {
                    if editor.id.is_some() {
                        if editor.occurrence.is_some() {
                            // As Outlook's "Verwijderen" of a series: this time, or the whole series.
                            let r = helder::menu_button(ui, t, Some(Icon::Trash), RichText::new("Verwijderen").size(14.0).color(t.text), |ui| {
                                ui.set_min_width(200.0);
                                if ui
                                    .add(egui::Button::image_and_text(Icon::CalendarX.image(helder::ICON, t.text), SKIP_ONE))
                                    .on_hover_text("Alleen deze keer vervalt; de reeks blijft.")
                                    .clicked()
                                {
                                    skip = true;
                                    ui.close();
                                }
                                if ui.add(egui::Button::image_and_text(Icon::Trash.image(helder::ICON, t.text), DELETE_SERIES)).clicked() {
                                    delete = true;
                                    ui.close();
                                }
                            });
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, DELETE_MENU));
                        } else if calls_off {
                            delete = helder::tool_button(ui, t, Icon::CalendarX, crate::meeting::CANCEL_MEETING)
                                .on_hover_text("Haalt de vergadering uit je agenda en mailt de genodigden een annulering.")
                                .clicked();
                        } else {
                            delete = helder::tool_button(ui, t, Icon::Trash, "Verwijderen").clicked();
                        }
                    }
                    close = helder::tool_button(ui, t, Icon::X, "Annuleren").clicked();
                });
            });
        });
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            helder::card(ui, t, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(580.0);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    form(ui, t, editor, &app.engine.session, &calendars, &local_calendars);
                });
            });
            if let Some((day, busy)) = &preview {
                helder::card(ui, t, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(250.0);
                        day_preview(ui, t, editor, *day, busy);
                    });
                });
            }
        });
    });
    close |= !open;
    if skip && let (Some(id), Some(start)) = (editor.id.clone(), editor.occurrence) {
        match app.engine.run("appointment.skip", &json!({ "id": id, "start": start })) {
            Ok(()) => close = true,
            Err(e) => editor.error = Some(describe(&e)),
        }
    }
    if save {
        editor.chip_typed(false, true);
        editor.chip_typed(true, true);
        match editor.params().and_then(|p| editor.all_attendees().map(|a| (p, a))) {
            Ok((params, people)) => {
                let calendar = editor.calendar_id.clone().or_else(|| calendars.first().map(|(id, _, _)| id.clone()));
                match save_meeting(app, params, people, action, organiser, stored.as_ref(), calendar.as_deref()) {
                    Ok(()) => close = true,
                    Err(e) => {
                        if let Some(editor) = app.calendar.editor.as_mut() {
                            editor.error = Some(e);
                        }
                    }
                }
            }
            Err(e) => editor.error = Some(e),
        }
    }
    let Some(editor) = app.calendar.editor.as_mut() else { return };
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
        let calendar = editor.calendar_id.clone();
        match app.engine.run("appointment.delete", &json!({ "id": id })) {
            Ok(()) => {
                // A meeting called off: everyone hears it, from the account it went out from.
                if calls_off
                    && let Some(a) = &stored
                    && let Some(account) = crate::meeting::account_for(&app.engine.session, calendar.as_deref())
                    && let Some(d) = crate::meeting::cancellation(a, &a.meeting.clone().map(|m| m.attendees).unwrap_or_default())
                {
                    app.engine.send(&account.id, d);
                }
                close = true;
            }
            Err(e) => {
                if let Some(editor) = app.calendar.editor.as_mut() {
                    editor.error = Some(describe(&e));
                }
            }
        }
    }
    if close {
        app.calendar.editor = None;
    }
}

/// Saves the window's appointment with its meeting and mails what the attendees must get: the
/// invitation, the update, and a cancellation to anyone taken off. Without a real account to
/// send from, nothing is saved, so nothing half happens.
fn save_meeting(
    app: &mut MailApp,
    mut params: serde_json::Value,
    people: Vec<Attendee>,
    action: MainAction,
    organiser: bool,
    stored: Option<&Appointment>,
    calendar: Option<&str>,
) -> Result<(), String> {
    let Some(editor) = app.calendar.editor.as_ref() else { return Ok(()) };
    if !organiser {
        // Someone else's meeting: the people are theirs to change.
        return app.engine.run("appointment.save", &params).map_err(|e| describe(&e));
    }
    let old = editor.meeting.clone();
    let account = if people.is_empty() && old.is_none() { None } else { crate::meeting::account_for(&app.engine.session, calendar) };
    if account.is_none() && (!people.is_empty() || action != MainAction::Save) {
        return Err(crate::meeting::NO_ACCOUNT.to_string());
    }
    // A meeting of an older version gets the UID its invitations went out with.
    let old = match (old, &account, stored) {
        (None, Some(acc), Some(s)) if editor.legacy => Some(Meeting {
            uid: crate::meeting::legacy_uid(s, &acc.address),
            organizer: acc.address.email().to_string(),
            organizer_name: acc.address.display().to_string(),
            sequence: 0,
            attendees: editor.invited.clone(),
        }),
        (old, _, _) => old,
    };
    let moved = stored.is_some_and(|s| params["start"].as_i64() != Some(s.start) || params["end"].as_i64() != Some(s.end));
    let meeting = match (&account, people.is_empty()) {
        (Some(acc), false) => {
            let mut m = old.clone().unwrap_or_else(|| Meeting {
                uid: crate::meeting::new_uid(&acc.address),
                organizer: acc.address.email().to_string(),
                organizer_name: if acc.address.display() == acc.address.email() { String::new() } else { acc.address.display().to_string() },
                ..Meeting::default()
            });
            if action == MainAction::Update {
                m.sequence += 1;
            }
            // A new time asks everyone again, as Outlook does.
            m.attendees = people
                .iter()
                .cloned()
                .map(|a| if moved { Attendee { response: merel_model::Response::None, replied: None, proposed: None, ..a } } else { a })
                .collect();
            Some(m)
        }
        _ => None,
    };
    params["meeting"] = json!(meeting);
    let removed: Vec<Attendee> = old
        .as_ref()
        .map(|m| m.attendees.iter().filter(|a| !people.iter().any(|p| p.email.eq_ignore_ascii_case(&a.email))).cloned().collect())
        .unwrap_or_default();
    app.engine.run("appointment.save", &params).map_err(|e| describe(&e))?;
    let Some(account) = account else { return Ok(()) };
    let saved = app.engine.session.selected_appointment().cloned();
    if let Some(a) = &saved
        && action != MainAction::Save
        && let Some(d) = crate::meeting::request(a, action == MainAction::Update)
    {
        app.engine.send(&account.id, d);
    }
    if action == MainAction::Update
        && let Some(a) = saved
        && let Some(d) = crate::meeting::cancellation(&Appointment { meeting: old.map(|m| Meeting { sequence: m.sequence + 1, ..m }), ..a }, &removed)
    {
        app.engine.send(&account.id, d);
    }
    Ok(())
}

/// The form of the appointment window, one row per subject with its icon before it.
fn form(
    ui: &mut Ui,
    t: &Tokens,
    editor: &mut Editor,
    session: &merel_engine::Session,
    calendars: &[(String, String, Color32)],
    local_calendars: &[String],
) {
    let wide = ui.available_width() - helder::ROW_ICON - 12.0;
    // As Outlook says over an appointment that is over.
    if editor.end().is_some_and(|end| end < Local::now().naive_local()) {
        egui::Frame::new().fill(t.warning.gamma_multiply(0.18)).corner_radius(8).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(Icon::Info.image(helder::ICON, t.text));
                ui.label(RichText::new(IN_THE_PAST).color(t.text));
            });
        });
        ui.add_space(4.0);
    }
    // Which calendar: a choice for a new appointment when there is more than one, a fact
    // otherwise.
    let current = editor.calendar_id.clone().or_else(|| calendars.first().map(|(id, _, _)| id.clone()));
    if let Some((_, name, color)) = calendars.iter().find(|(id, _, _)| Some(id) == current.as_ref()) {
        helder::dot_row(ui, *color, |ui| {
            if editor.id.is_none() && calendars.len() > 1 {
                helder::quiet(ui, t, |ui| {
                    helder::menu_button(ui, t, None, RichText::new(name).size(14.0).color(t.text), |ui| {
                        for (id, n, c) in calendars {
                            if ui.add(egui::Button::new(helder::dotted(n, *c, t)).selected(current.as_ref() == Some(id))).clicked() {
                                editor.calendar_id = Some(id.clone());
                                ui.close();
                            }
                        }
                    })
                    .on_hover_text("Agenda");
                });
            } else {
                ui.label(RichText::new(name).size(14.0).color(t.text));
            }
        });
    }
    helder::icon_row(ui, t, Icon::Type, |ui| helder::underline_field(ui, t, &mut editor.title, "Titel toevoegen", wide, true));
    // The categories as chips that take one off, then the menu that puts them on.
    let mut off = None;
    helder::icon_row(ui, t, Icon::Tag, |ui| {
        helder::quiet(ui, t, |ui| {
            ui.horizontal_wrapped(|ui| {
                for name in &editor.categories {
                    let color = crate::categories::color_of(session.category_color(name));
                    let r = ui.add(egui::Button::new(helder::dotted(&format!("{name}  ✕"), color, t)).frame_when_inactive(false));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, remove_category_label(name)));
                    if r.on_hover_text("Klik om deze categorie weg te halen").clicked() {
                        off = Some(name.clone());
                    }
                }
                category_menu(ui, t, editor, session);
            });
        });
    });
    if let Some(name) = off {
        editor.categories.retain(|c| *c != name);
    }
    // The people, as chips on the line where they were added, with what they answered.
    let organiser = editor.meeting.as_ref().is_none_or(|m| crate::meeting::organised_here(session, m));
    let mut off = helder::icon_row(ui, t, Icon::Users, |ui| attendee_line(ui, t, editor, false, organiser, wide));
    if organiser && !editor.attendees.trim().is_empty() {
        ui.horizontal(|ui| {
            helder::indent(ui);
            ui.vertical(|ui| crate::contacts::complete_addresses(ui, t, session, &mut editor.attendees));
        });
        editor.chip_typed(false, false);
    }
    if editor.show_optional || editor.invited.iter().any(|a| a.optional) {
        ui.horizontal(|ui| {
            helder::indent(ui);
            ui.label(RichText::new(crate::meeting::OPTIONAL).size(13.0).color(t.text_dim));
            off = off.take().or(attendee_line(ui, t, editor, true, organiser, wide - 80.0));
        });
        if organiser && !editor.optional.trim().is_empty() {
            ui.horizontal(|ui| {
                helder::indent(ui);
                ui.vertical(|ui| crate::contacts::complete_addresses(ui, t, session, &mut editor.optional));
            });
            editor.chip_typed(true, false);
        }
    }
    if let Some(email) = off {
        editor.invited.retain(|a| a.email != email);
    }
    // Who answered what, and a new time someone asked for, as Outlook's tracking.
    if let Some(m) = &editor.meeting
        && !m.attendees.is_empty()
    {
        helder::note(ui, t, &format!("Reacties: {}.", crate::meeting::tally_line(m)));
    }
    let proposals: Vec<(String, i64, i64)> = editor.invited.iter().filter_map(|a| a.proposed.map(|(s, e)| (a.label().to_string(), s, e))).collect();
    for (who, start, end) in proposals {
        let when = crate::invitation::when(&Appointment {
            id: String::new(),
            calendar_id: String::new(),
            title: String::new(),
            start,
            end,
            all_day: false,
            location: String::new(),
            notes: String::new(),
            notes_rich: None,
            recurrence: None,
            reminder: None,
            private: false,
            show_as: ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
        });
        ui.horizontal_wrapped(|ui| {
            helder::indent(ui);
            ui.label(RichText::new(format!("{who} stelt een nieuwe tijd voor: {when}.")).size(12.0).color(t.text));
            if organiser
                && ui
                    .add(egui::Button::new(RichText::new(TAKE_PROPOSAL).size(13.0).color(t.accent_text)).frame_when_inactive(false))
                    .on_hover_text("Zet deze tijd in het venster; met Update verzenden hoort iedereen het.")
                    .clicked()
            {
                editor.set_times(start, end);
                editor.all_day = false;
            }
        });
    }
    helder::icon_row(ui, t, Icon::Clock, |ui| {
        let before = parse_date(&editor.start_date);
        if let Some(start) = crate::date_picker::field(ui, t, &mut editor.start_date, "19-10-2026", 96.0, START_PICKER)
            && let (Some(before), Some(end)) = (before, parse_date(&editor.end_date))
        {
            // The end moves along, so the appointment keeps its length.
            editor.end_date = add_days(end, (start - before).num_days()).format(DATE_FORMAT).to_string();
        }
        if !editor.all_day
            && let Some(time) = time_field(ui, t, &mut editor.start_time, "start")
        {
            editor.move_start(&time);
        }
        ui.label(RichText::new("tot").color(t.text_dim));
        crate::date_picker::field(ui, t, &mut editor.end_date, "19-10-2026", 96.0, END_PICKER);
        if !editor.all_day
            && let Some(time) = time_field(ui, t, &mut editor.end_time, "end")
        {
            editor.end_time = time;
        }
    });
    // On a row of its own: beside the two dates with their calendar buttons it has no room.
    ui.horizontal(|ui| {
        helder::indent(ui);
        helder::toggle(ui, t, &mut editor.all_day, "Hele dag");
    });
    // A calendar on this computer keeps a series as its rule. An account's server owns its
    // own series: a new appointment there can start one, but what comes back is listed and
    // edited one occurrence at a time.
    let local = current.as_ref().is_some_and(|id| local_calendars.contains(id));
    if local || editor.id.is_none() {
        repeat_fields(ui, t, editor);
        if !local && editor.repeat.is_some() {
            helder::note(ui, t, "De reeks wordt in het account aangemaakt; daarna toont Merel elke keer apart en bewerk je ze los van elkaar.");
        }
        if local && let Some(occurrence) = editor.occurrence {
            let when = to_local(occurrence);
            let text = format!(
                "Geopend vanaf de keer op {} {} {}. Wijzigingen gelden voor de hele reeks.",
                weekday_name(when.date()),
                when.day(),
                month_name(when.month())
            );
            helder::note(ui, t, &text);
        }
    } else {
        editor.repeat = None;
    }
    helder::icon_row(ui, t, Icon::MapPin, |ui| helder::underline_field(ui, t, &mut editor.location, "Locatie toevoegen", wide, false));
    ui.add_space(4.0);
    // The notes with Outlook's formatting bar over them, as in the message window.
    ui.add_space(6.0);
    helder::notes_box(ui, t, &mut editor.notes_editor, &mut editor.notes, wide, 140.0);
    if let Some(e) = &editor.error {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            helder::indent(ui);
            ui.add(egui::Label::new(RichText::new(e).color(t.flag)).wrap());
        });
    }
}

/// The form's "Categoriseren" menu: every known category with its colour, ticked when the
/// appointment has it, the ones it carries that this computer does not know, and
/// "Alle categorieën wissen".
fn category_menu(ui: &mut Ui, t: &Tokens, editor: &mut Editor, session: &merel_engine::Session) {
    let mut names: Vec<String> = session.categories.iter().map(|c| c.name.clone()).collect();
    names.extend(editor.categories.iter().filter(|c| !names.iter().any(|n| n.eq_ignore_ascii_case(c))).cloned().collect::<Vec<_>>());
    let label = if editor.categories.is_empty() { CATEGORIZE } else { "Nog een categorie" };
    let r = helder::menu_button(ui, t, None, RichText::new(label).size(14.0).color(t.text_dim), |ui| {
        for name in &names {
            let on = editor.categories.iter().any(|c| c.eq_ignore_ascii_case(name));
            let color = crate::categories::color_of(session.category_color(name));
            if ui.add(egui::Button::new(helder::dotted(name, color, t)).selected(on)).clicked() {
                if on {
                    editor.categories.retain(|c| !c.eq_ignore_ascii_case(name));
                } else {
                    editor.categories.push(name.clone());
                }
            }
        }
        if names.is_empty() {
            ui.label(RichText::new("Nog geen categorieën; maak ze bij E-mail via Categoriseren.").size(12.0).color(t.text_dim));
        }
        ui.separator();
        if ui.add_enabled(!editor.categories.is_empty(), egui::Button::new(CLEAR_CATEGORIES)).clicked() {
            editor.categories.clear();
            ui.close();
        }
    });
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, CATEGORIZE));
}

/// The appointment window's category menu, as screen readers name it.
pub const CATEGORIZE: &str = "Categorie toevoegen";
/// Its last item.
pub const CLEAR_CATEGORIES: &str = "Alle categorieën wissen";

/// The label of the button that takes `name` off the appointment in its window.
pub fn remove_category_label(name: &str) -> String {
    format!("Categorie {name} weghalen")
}

/// A failed change as a notice: a store that cannot write is an error, the rest the person can mend.
fn notice(e: &CommandError) -> crate::Notice {
    match e {
        CommandError::Store(_) => crate::Notice::error(describe(e)),
        _ => crate::Notice::warning(describe(e)),
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
            notes_rich: None,
            recurrence: None,
            reminder: None,
            private: false,
            show_as: merel_model::ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
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
    fn prive_goes_through_the_window_and_shows_a_lock() {
        let mut e = Editor::new_at(d(2026, 10, 21).and_hms_opt(14, 0, 0).unwrap());
        e.title = "Dokter".into();
        assert_eq!(e.params().unwrap()["private"], json!(false));
        e.private = true;
        assert_eq!(e.params().unwrap()["private"], json!(true));
        let a = Appointment {
            id: "a".into(),
            calendar_id: "local".into(),
            title: "Dokter".into(),
            start: 1_792_404_000,
            end: 1_792_407_600,
            all_day: false,
            location: String::new(),
            notes: String::new(),
            notes_rich: None,
            recurrence: None,
            reminder: None,
            private: true,
            show_as: merel_model::ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
        };
        assert!(Editor::from_appointment(&a, None).private);
        assert_eq!(chip_title(&a), "🔒 Dokter");
        assert_eq!(e.params().unwrap()["show_as"], json!("busy"), "busy unless chosen otherwise");
        e.show_as = ShowAs::Oof;
        assert_eq!(e.params().unwrap()["show_as"], json!("oof"));
        let free = Appointment { show_as: ShowAs::Free, ..a.clone() };
        assert_eq!(Editor::from_appointment(&free, None).show_as, ShowAs::Free);
        assert_eq!((show_as_label(ShowAs::Free), show_as_label(ShowAs::Tentative), show_as_label(ShowAs::Oof)), ("Vrij", "Voorlopig", "Afwezig"));
        let blue = Color32::from_rgb(0, 0, 255);
        let t = Tokens::light();
        let free = chip_paint(blue, ShowAs::Free, false, &t);
        assert!(free.fill == t.card && free.border.is_some(), "a free block is the card's white with a border");
        let busy = chip_paint(blue, ShowAs::Busy, false, &t);
        assert!(busy.fill != t.card && busy.border.is_none() && busy.stripe == Some(blue), "a busy block is tinted with a bar");
        assert_eq!(chip_paint(blue, ShowAs::Oof, true, &t).stripe, Some(OOF_COLOR), "out of office is purple, whatever the calendar's colour");
        assert_eq!(chip_title(&Appointment { recurrence: Some(Recurrence::every(Frequency::Weekly)), ..a.clone() }), "🔒 🔁 Dokter");
        assert_eq!(chip_title(&Appointment { private: false, ..a }), "Dokter");
    }

    #[test]
    fn appointments_at_the_same_time_stand_side_by_side() {
        assert!(lanes(&[]).is_empty());
        assert_eq!(lanes(&[(0, 10)]), vec![(0, 1)]);
        // 9-11 and 10-12 overlap, 11-12 fits under the first; 13-14 stands alone.
        assert_eq!(lanes(&[(9, 11), (10, 12), (11, 12), (13, 14)]), vec![(0, 2), (1, 2), (0, 2), (0, 1)]);
        assert_eq!(lanes(&[(0, 5), (0, 5), (0, 5)]), vec![(0, 3), (1, 3), (2, 3)], "three at once take a third each");
        assert_eq!(lanes(&[(0, 5), (5, 6)]), vec![(0, 1), (0, 1)], "one that starts as the other ends does not share");
    }

    #[test]
    fn appointment_text_meets_wcag_aa_on_every_block() {
        // Calendar colours as servers and Merel hand them out, from pale to deep.
        let colors = [
            Color32::from_rgb(31, 111, 201),
            Color32::from_rgb(46, 125, 91),
            Color32::from_rgb(255, 200, 0),
            Color32::from_rgb(220, 40, 40),
            Color32::from_rgb(255, 140, 0),
            Color32::from_rgb(0, 0, 255),
            Color32::from_rgb(160, 160, 160),
            Color32::WHITE,
            Color32::BLACK,
        ];
        let picked = merel_model::CALENDAR_COLORS.map(|(_, c)| Color32::from_rgb(c[0], c[1], c[2]));
        let categories = merel_model::CategoryColor::ALL.map(crate::categories::color_of);
        for t in [Tokens::light(), Tokens::dark()] {
            for c in colors.into_iter().chain(picked).chain(categories) {
                // The calendar's tab: its name on a tint, its buttons' icons on a deeper one.
                let tab = crate::theme::contrast(t.text, crate::theme::over(c, t.card, 0.25));
                assert!(tab >= 4.5, "tab {c:?} dark {}: {tab}", t.dark);
                let icon = crate::theme::contrast(t.text, crate::theme::over(c, t.card, 0.45));
                assert!(icon >= 3.0, "tab button {c:?} dark {}: {icon}", t.dark);
                for show_as in [ShowAs::Free, ShowAs::Tentative, ShowAs::Busy, ShowAs::Oof] {
                    for hovered in [false, true] {
                        let look = chip_paint(c, show_as, hovered, &t);
                        let ratio = crate::theme::contrast(look.text, look.fill);
                        assert!(ratio >= 4.5, "{c:?} {show_as:?} hovered {hovered} dark {}: {ratio}", t.dark);
                    }
                }
            }
        }
    }

    #[test]
    fn moving_the_start_keeps_the_length_and_the_list_offers_half_hours() {
        let mut e = Editor::new_at(d(2026, 10, 21).and_hms_opt(14, 0, 0).unwrap());
        e.end_time = "15:30".into();
        e.move_start("23:00");
        assert_eq!((e.start_time.as_str(), e.end_date.as_str(), e.end_time.as_str()), ("23:00", "22-10-2026", "00:30"));
        e.end_time = "nonsense".into();
        e.move_start("09:00");
        assert_eq!((e.start_time.as_str(), e.end_time.as_str()), ("09:00", "nonsense"), "an end that does not parse is left alone");
        let times: Vec<String> = half_hours().collect();
        assert_eq!((times.len(), times[0].as_str(), times[19].as_str(), times[47].as_str()), (48, "00:00", "09:30", "23:30"));
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
            notes_rich: None,
            recurrence: None,
            reminder: None,
            private: false,
            show_as: merel_model::ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
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
    fn the_working_day_runs_from_eight_to_five_on_weekdays() {
        let monday = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let saturday = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        assert!(outside_work(monday, 7) && !outside_work(monday, 8) && !outside_work(monday, 16) && outside_work(monday, 17));
        assert!(outside_work(saturday, 10), "the weekend is shaded whole");
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
            notes_rich: None,
            recurrence: Some(Recurrence { interval: 2, count: Some(8), ..Recurrence::every(Frequency::Monthly) }),
            reminder: None,
            private: false,
            show_as: merel_model::ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
        };
        let back = Editor::from_appointment(&a, Some(a.start + 86_400 * 61));
        assert_eq!((back.repeat, back.interval.as_str(), back.ends, back.count.as_str()), (Some(Frequency::Monthly), "2", Ends::AfterCount, "8"));
        assert_eq!(back.occurrence, Some(a.start + 86_400 * 61));
        let p = back.params().unwrap();
        assert_eq!(p["recurrence"]["count"], 8);
        assert_eq!(p["id"], "x");
    }
}
