//! The meetings module: the online meetings of today or the coming week, found in the
//! appointments by their link (see `merel_model::MeetingLink`), with a button to join; and a
//! place to paste a link someone sent. Joining opens the link in the browser, which hands it
//! to the service's own app when one is installed.

use chrono::{Local, NaiveDate};
use egui::{FontId, Margin, RichText, Sense, Ui, vec2};

use merel_model::{Appointment, MeetingLink, Response, meeting_link};

use crate::calendar::{add_days, month_name, to_local, to_timestamp, today, weekday_name};
use crate::helder::{self, Icon};
use crate::i18n::{tr, tr_args};
use crate::{MailApp, Tokens};

/// How long before the start the list offers to join, in seconds.
const JOIN_EARLY: i64 = 15 * 60;

/// Which meetings the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Range {
    #[default]
    Today,
    Week,
}

/// What the meetings module remembers between frames.
#[derive(Default)]
pub struct State {
    pub range: Range,
    /// The meeting on show: the appointment's id and the start of the occurrence.
    pub selected: Option<(String, i64)>,
    /// A link pasted to join.
    pub paste: String,
    pub paste_error: Option<String>,
    /// When a link was last copied (egui time), to say so for a moment.
    pub copied_at: Option<f64>,
}

/// Where a meeting stands: how long until it starts, going on now, or over.
fn status(a: &Appointment, now: i64) -> String {
    if a.end <= now {
        tr("meet-ended")
    } else if a.start <= now {
        tr("meet-now")
    } else if a.start - now < 3_600 {
        tr_args("meet-starts-in", &[("minutes", ((a.start - now + 59) / 60).into())])
    } else {
        String::new()
    }
}

/// Whether the meeting can be joined now: going on, or starting within a quarter of an hour.
fn joinable(a: &Appointment, now: i64) -> bool {
    a.end > now && a.start - now <= JOIN_EARLY
}

/// The service's name, or "" for a link of a site Merel does not know.
fn service(link: &MeetingLink) -> &'static str {
    link.service.name()
}

impl MailApp {
    /// The online meetings the list shows, earliest first.
    fn meetings(&self) -> Vec<(Appointment, MeetingLink)> {
        let day = today();
        let days = if self.meet.range == Range::Today { 1 } else { 7 };
        let (Some(from), Some(to)) = (midnight(day), midnight(add_days(day, days))) else { return Vec::new() };
        self.engine.session.online_meetings(from, to)
    }

    /// Opens a meeting's link in the browser.
    fn join(&mut self, ctx: &egui::Context, url: &str) {
        if let Some(link) = meeting_link(url) {
            ctx.open_url(egui::OpenUrl::new_tab(link.url));
        }
    }

    /// Joins with the link pasted on the start page.
    fn join_pasted(&mut self, ctx: &egui::Context) {
        match meeting_link(&self.meet.paste) {
            Some(link) => {
                self.meet.paste.clear();
                self.meet.paste_error = None;
                self.join(ctx, &link.url);
            }
            None => self.meet.paste_error = Some(tr("meet-paste-bad")),
        }
    }

    /// Opens the window for a new appointment at the next whole hour.
    pub fn plan_meeting(&mut self) {
        let now = to_local(Local::now().timestamp());
        let hour = now.format("%H").to_string().parse::<u32>().unwrap_or(8) + 1;
        if hour > 23 {
            self.new_appointment(add_days(now.date(), 1), 9);
        } else {
            self.new_appointment(now.date(), hour);
        }
    }
}

fn midnight(d: NaiveDate) -> Option<i64> {
    to_timestamp(d.and_hms_opt(0, 0, 0)?)
}

pub fn show(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    nav_pane(app, ui, t);
    let list = app.meetings();
    list_pane(app, ui, t, &list);
    let frame = helder::card_frame(t).inner_margin(Margin::same(20)).outer_margin(Margin { left: 6, right: 12, top: 0, bottom: 12 });
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        let open = app.meet.selected.as_ref().and_then(|(id, start)| list.iter().find(|(a, _)| a.id == *id && a.start == *start)).cloned();
        match open {
            Some((a, link)) => details(app, ui, t, &a, &link),
            None => start_page(app, ui, t),
        }
    });
}

/// The side menu: today or the coming week.
fn nav_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("meet-nav")
        .exact_size(210.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().inner_margin(Margin { left: 0, right: 8, top: 0, bottom: 12 }))
        .show(ui, |ui| {
            for (key, range, icon) in [("meet-today", Range::Today, Icon::CalendarCheck), ("meet-week", Range::Week, Icon::CalendarRange)] {
                if helder::nav_item(ui, t, icon, &tr(key), app.meet.range == range).clicked() {
                    app.meet.range = range;
                    app.meet.selected = None;
                }
            }
        });
}

/// The meetings, a heading per day.
fn list_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens, list: &[(Appointment, MeetingLink)]) {
    egui::Panel::left("meet-list")
        .default_size(340.0)
        .resizable(true)
        .show_separator_line(false)
        .frame(helder::card_frame(t).inner_margin(Margin::same(14)).outer_margin(Margin { left: 0, right: 6, top: 0, bottom: 12 }))
        .show(ui, |ui| {
            let title = if app.meet.range == Range::Today { tr("meet-today") } else { tr("meet-week") };
            ui.label(RichText::new(title).size(18.0).color(t.text));
            if app.meet.range == Range::Today {
                ui.label(RichText::new(day_name(today())).size(12.5).color(t.text_dim));
            }
            ui.add_space(10.0);
            if list.is_empty() {
                let key = if app.meet.range == Range::Today { "meet-none-today" } else { "meet-none-week" };
                ui.label(RichText::new(tr(key)).color(t.text_dim));
            }
            let now = Local::now().timestamp();
            let mut pick = None;
            let mut join = None;
            egui::ScrollArea::vertical().auto_shrink(false).max_height(ui.available_height() - 48.0).show(ui, |ui| {
                let mut day = None;
                for (a, link) in list {
                    let date = to_local(a.start).date();
                    if app.meet.range == Range::Week && day != Some(date) {
                        day = Some(date);
                        ui.add_space(4.0);
                        ui.label(RichText::new(day_name(date)).size(12.5).strong().color(t.text_dim));
                    }
                    let (row, joined) = meeting_row(app, ui, t, a, link, now);
                    if joined {
                        join = Some(link.url.clone());
                    } else if row.clicked() {
                        pick = Some((a.id.clone(), a.start));
                    }
                }
            });
            if let Some(sel) = pick {
                app.meet.selected = Some(sel);
            }
            if let Some(url) = join {
                app.join(ui.ctx(), &url);
            }
            ui.add_space(8.0);
            helder::icon_row(ui, t, Icon::Info, |ui| {
                ui.label(RichText::new(tr("meet-found-note")).size(12.0).color(t.text_dim));
            });
        });
}

/// "woensdag 21 oktober".
fn day_name(d: NaiveDate) -> String {
    use chrono::Datelike;
    format!("{} {} {}", weekday_name(d), d.day(), month_name(d.month()))
}

/// One meeting in the list: its times, a bar in its calendar's colour, the title, who comes,
/// where it stands, and a button to join once it is close. Gives the row's response and
/// whether its join button was clicked.
fn meeting_row(app: &MailApp, ui: &mut Ui, t: &Tokens, a: &Appointment, link: &MeetingLink, now: i64) -> (egui::Response, bool) {
    let on = app.meet.selected.as_ref().is_some_and(|(id, start)| *id == a.id && *start == a.start);
    let can_join = joinable(a, now);
    let height = if can_join { 84.0 } else { 64.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let state = status(a, now);
    let (start, end) = (to_local(a.start).format("%H:%M").to_string(), to_local(a.end).format("%H:%M").to_string());
    // A screen reader hears what the row shows: the title, the times and where it stands.
    let heard = [a.title.clone(), format!("{start}–{end}"), state.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, &heard));
    let p = ui.painter();
    if on || r.hovered() {
        p.rect_filled(rect, 8, if on { t.checked } else { t.hover });
    }
    let dim = if a.end <= now { t.text_dim } else { t.text };
    let x = rect.left() + 10.0;
    p.text(egui::pos2(x, rect.top() + 10.0), egui::Align2::LEFT_TOP, start, FontId::proportional(14.0), dim);
    p.text(egui::pos2(x, rect.top() + 28.0), egui::Align2::LEFT_TOP, end, FontId::proportional(12.5), t.text_dim);
    let color = app.engine.session.calendar(&a.calendar_id).map(|c| egui::Color32::from_rgb(c.color[0], c.color[1], c.color[2])).unwrap_or(t.accent);
    let bar = rect.left() + 58.0;
    p.rect_filled(egui::Rect::from_min_max(egui::pos2(bar, rect.top() + 8.0), egui::pos2(bar + 3.0, rect.bottom() - 8.0)), 1.5, color);
    let x = bar + 12.0;
    let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(rect.right() - 8.0, rect.bottom()));
    let p = p.with_clip_rect(clip);
    p.text(egui::pos2(x, rect.top() + 9.0), egui::Align2::LEFT_TOP, &a.title, FontId::proportional(14.0), dim);
    let mut line = who(a);
    let name = service(link);
    if !name.is_empty() {
        line = if line.is_empty() { name.to_string() } else { format!("{line} · {name}") };
    }
    p.text(egui::pos2(x, rect.top() + 29.0), egui::Align2::LEFT_TOP, line, FontId::proportional(12.5), t.text_dim);
    if !state.is_empty() {
        p.text(egui::pos2(x, rect.top() + 47.0), egui::Align2::LEFT_TOP, state, FontId::proportional(12.0), t.accent_text);
    }
    let mut joined = false;
    if can_join {
        let size = vec2(110.0, 28.0);
        let at = egui::Rect::from_min_size(egui::pos2(rect.right() - size.x - 8.0, rect.bottom() - size.y - 8.0), size);
        let label = tr_args("meet-join-named", &[("title", a.title.as_str().into())]);
        let b = ui.put(at, helder::primary_text(t, &tr("meet-join")));
        b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        joined = b.clicked();
    }
    (r, joined)
}

/// "Sanne de Vries · 5 personen": the organiser and how many are invited, or "".
fn who(a: &Appointment) -> String {
    let Some(m) = &a.meeting else { return String::new() };
    let organizer = if m.organizer_name.is_empty() { m.organizer.clone() } else { m.organizer_name.clone() };
    let count = m.attendees.len() + 1;
    tr_args("meet-who", &[("organizer", organizer.into()), ("count", count.into())])
}

/// The meeting on show: when, how to join, who is invited, the notes.
fn details(app: &mut MailApp, ui: &mut Ui, t: &Tokens, a: &Appointment, link: &MeetingLink) {
    let now = Local::now().timestamp();
    ui.horizontal(|ui| {
        let state = status(a, now);
        if !state.is_empty() {
            ui.label(RichText::new(state).size(12.5).color(t.accent_text));
        }
        let name = service(link);
        if !name.is_empty() {
            ui.label(RichText::new(name).size(12.5).color(t.text_dim));
        }
    });
    ui.add_space(4.0);
    ui.label(RichText::new(&a.title).size(22.0).color(t.text));
    ui.add_space(10.0);
    helder::icon_row(ui, t, Icon::Clock, |ui| {
        let (start, end) = (to_local(a.start), to_local(a.end));
        let line = format!("{} · {}–{}", day_name(start.date()), start.format("%H:%M"), end.format("%H:%M"));
        ui.label(RichText::new(line).color(t.text));
    });
    helder::icon_row(ui, t, Icon::Video, |ui| {
        let name = service(link);
        let via = if name.is_empty() { tr("meet-via-link") } else { tr_args("meet-via", &[("service", name.into())]) };
        ui.label(RichText::new(via).color(t.text));
    });
    if !a.location.is_empty() && a.location.trim() != link.url {
        helder::icon_row(ui, t, Icon::MapPin, |ui| {
            ui.label(RichText::new(&a.location).color(t.text));
        });
    }
    if let Some(m) = &a.meeting {
        helder::icon_row(ui, t, Icon::Users, |ui| {
            ui.horizontal_wrapped(|ui| {
                let organizer = if m.organizer_name.is_empty() { m.organizer.as_str() } else { m.organizer_name.as_str() };
                ui.label(RichText::new(tr_args("meet-organizer", &[("name", organizer.into())])).color(t.text));
                for p in &m.attendees {
                    let name = if p.name.is_empty() { p.email.as_str() } else { p.name.as_str() };
                    let answer = match p.response {
                        Response::Accepted => "✓ ",
                        Response::Declined => "✗ ",
                        _ => "",
                    };
                    ui.label(RichText::new(format!("· {answer}{name}")).color(t.text_dim));
                }
            });
        });
    }
    let notes = a.notes.lines().map(str::trim).filter(|l| !l.is_empty()).take(4).collect::<Vec<_>>().join("\n");
    if !notes.is_empty() {
        helder::icon_row(ui, t, Icon::Notes, |ui| {
            ui.add(egui::Label::new(RichText::new(notes).color(t.text_dim)).wrap());
        });
    }
    ui.add_space(12.0);
    helder::rule(ui, t);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if helder::primary_button(ui, t, Icon::Video, &tr("meet-join")).clicked() {
            app.join(ui.ctx(), &link.url);
        }
        let now = ui.input(|i| i.time);
        let copied = app.meet.copied_at.is_some_and(|at| now - at < 2.0);
        let copy = if copied { tr("meet-copied") } else { tr("meet-copy") };
        if helder::secondary(ui, t, |ui| helder::tool_button(ui, t, Icon::Copy, &copy)).clicked() {
            ui.ctx().copy_text(link.url.clone());
            app.meet.copied_at = Some(now);
        }
    });
    ui.add_space(6.0);
    ui.add(egui::Label::new(RichText::new(&link.url).font(FontId::monospace(11.5)).color(t.text_dim)).wrap());
    ui.add_space(6.0);
    helder::note(ui, t, &tr("meet-browser-note"));
}

/// What the right side shows without a meeting picked: joining with a pasted link.
fn start_page(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    ui.label(RichText::new(tr("meet-title")).size(20.0).color(t.text));
    ui.add_space(8.0);
    helder::paragraph(ui, t, &tr("meet-intro"));
    ui.add_space(16.0);
    ui.label(RichText::new(tr("meet-paste-title")).size(16.0).color(t.text));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 150.0).clamp(200.0, 520.0);
        let r =
            ui.add(egui::TextEdit::singleline(&mut app.meet.paste).id(egui::Id::new(PASTE)).desired_width(width).hint_text(tr("meet-paste-hint")));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, tr("meet-paste-title")));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let ready = !app.meet.paste.trim().is_empty();
        let clicked = ui.add_enabled_ui(ready, |ui| helder::primary_button(ui, t, Icon::Video, &tr("meet-join"))).inner.clicked();
        if ready && (clicked || enter) {
            app.join_pasted(ui.ctx());
        }
    });
    if let Some(e) = &app.meet.paste_error {
        ui.label(RichText::new(e).color(t.flag));
    }
    ui.add_space(8.0);
    helder::note(ui, t, &tr("meet-browser-note"));
    ui.add_space(16.0);
    if helder::secondary(ui, t, |ui| ui.button(tr("meet-plan"))).clicked() {
        app.plan_meeting();
    }
}

/// The field to paste a link in: its widget id, so the ribbon can put the cursor there.
pub(crate) const PASTE: &str = "meet-paste";

/// The ribbon of the meetings module.
pub fn ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    if crate::ribbon_action(ui, t, "📅", &tr("meet-plan"), true) {
        app.plan_meeting();
    }
    if crate::ribbon_action(ui, t, "🔗", &tr("meet-join-link"), true) {
        app.meet.selected = None;
        ui.memory_mut(|m| m.request_focus(egui::Id::new(PASTE)));
    }
    crate::ribbon_separator(ui);
    for (key, range) in [("meet-today", Range::Today), ("meet-week", Range::Week)] {
        if crate::ribbon_toggle(ui, t, "", &tr(key), app.meet.range == range) {
            app.meet.range = range;
            app.meet.selected = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: i64, end: i64) -> Appointment {
        Appointment {
            id: "a".into(),
            calendar_id: "local".into(),
            title: "Overleg".into(),
            start,
            end,
            all_day: false,
            location: String::new(),
            notes: String::new(),
            recurrence: None,
            reminder: None,
            private: false,
            show_as: merel_model::ShowAs::Busy,
            categories: Vec::new(),
            notes_rich: None,
            meeting: None,
            online: None,
            make_online: false,
        }
    }

    #[test]
    fn a_meeting_can_be_joined_from_a_quarter_before_until_its_end() {
        let now = 100_000;
        assert!(!joinable(&at(now + 20 * 60, now + 80 * 60), now));
        assert!(joinable(&at(now + 15 * 60, now + 80 * 60), now));
        assert!(joinable(&at(now - 60, now + 60), now));
        assert!(!joinable(&at(now - 120, now), now));
        assert_eq!(status(&at(now + 4 * 60, now + 3_600), now), tr_args("meet-starts-in", &[("minutes", 4.into())]));
        assert_eq!(status(&at(now - 60, now + 60), now), tr("meet-now"));
        assert_eq!(status(&at(now - 120, now), now), tr("meet-ended"));
        assert_eq!(status(&at(now + 7_200, now + 9_000), now), "");
    }
}
