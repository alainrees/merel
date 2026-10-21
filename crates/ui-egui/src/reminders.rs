//! The reminders window: the appointments whose reminder is due, the flagged messages whose
//! follow-up day has come and the tasks whose due day has come, each to open, close or put off. It appears on its own, asks for the
//! window's attention once per reminder, and goes away when nothing is due.

use std::collections::HashSet;

use chrono::Datelike;
use egui::{Align2, RichText, vec2};
use serde_json::json;

use crate::calendar::{MONTHS, WEEKDAYS, to_local};
use crate::{MailApp, Module, Tokens, helder, notifications};

/// How long a reminder can be put off, as the window offers it: minutes and label.
const SNOOZES: [(u32, &str); 5] = [(5, "5 minuten"), (15, "15 minuten"), (60, "1 uur"), (240, "4 uur"), (1440, "1 dag")];

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "Begint over 10 minuten", "Begint nu", "Begon 2 uur geleden".
pub fn relative(now: i64, start: i64) -> String {
    let diff = start - now;
    let amount = |secs: i64| -> String {
        let secs = secs.abs();
        match secs {
            s if s < 3600 => {
                let m = (s / 60).max(1);
                if m == 1 { "1 minuut".into() } else { format!("{m} minuten") }
            }
            s if s < 86_400 => format!("{} uur", s / 3600),
            s => {
                let d = s / 86_400;
                if d == 1 { "1 dag".into() } else { format!("{d} dagen") }
            }
        }
    };
    if diff.abs() < 60 {
        "Begint nu".into()
    } else if diff > 0 {
        format!("Begint over {}", amount(diff))
    } else {
        format!("Begon {} geleden", amount(diff))
    }
}

/// "maandag 19 oktober om 09:00", or "maandag 19 oktober, hele dag".
fn when(start: i64, all_day: bool) -> String {
    let t = to_local(start);
    let day = format!("{} {} {}", WEEKDAYS[t.weekday().num_days_from_monday() as usize], t.day(), MONTHS[(t.month0()) as usize]);
    if all_day { format!("{day}, hele dag") } else { format!("{day} om {}", t.format("%H:%M")) }
}

enum Action {
    Open(String, i64),
    Dismiss(String, i64),
    Snooze(String, i64, u32),
    /// On a follow-up: the message id.
    OpenMail(String),
    Done(String),
    DismissMail(String),
    SnoozeMail(String, u32),
    /// On a task: the task id.
    OpenTask(String),
    DoneTask(String),
    DismissTask(String),
    SnoozeTask(String, u32),
    DismissAll,
}

/// "Vandaag", "Gisteren" or "Verlopen op …" for a follow-up day seen from `today`.
fn follow_up_due(due: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    if due == today {
        "Vandaag".into()
    } else if Some(due) == today.pred_opt() {
        "Gisteren".into()
    } else {
        format!("Verlopen op {}", crate::dates::day_name(due))
    }
}

/// The reminder window's width, inside its frame.
const WIDTH: f32 = 420.0;

/// One reminder as a white card: a round badge with `icon`, what kind of reminder it is, the
/// title, when it is due in `lead`'s colour, the `details` each with its icon, and the buttons
/// `buttons` adds. A screen reader hears the title as "kind: title".
#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    t: &Tokens,
    icon: helder::Icon,
    kind: &str,
    title: &str,
    lead: (String, egui::Color32),
    details: &[(helder::Icon, String)],
    buttons: impl FnOnce(&mut egui::Ui),
) {
    helder::card_frame(t).inner_margin(14).show(ui, |ui| {
        ui.set_width(WIDTH - 28.0);
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 18.0, t.checked);
            icon.image(helder::ICON, t.accent_text).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(helder::ICON, helder::ICON)));
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(kind).size(12.0).color(t.text_dim));
                let name = format!("{kind}: {title}");
                ui.add(egui::Label::new(RichText::new(title).size(15.0).strong().color(t.text)).wrap())
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &name));
                ui.label(RichText::new(lead.0).size(13.0).color(lead.1));
                for (icon, text) in details {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.add(icon.image(14.0, t.text_dim));
                        ui.label(RichText::new(text).size(13.0).color(t.text_dim));
                    });
                }
            });
        });
        ui.add_space(6.0);
        helder::quiet(ui, t, |ui| {
            ui.spacing_mut().button_padding.x = 8.0;
            ui.horizontal(buttons);
        });
    });
}

/// The "Uitstellen" menu: how long to put a reminder off for, if one was picked.
fn snooze(ui: &mut egui::Ui, t: &Tokens) -> Option<u32> {
    let mut picked = None;
    helder::menu_button(ui, t, Some(helder::Icon::Clock), RichText::new("Uitstellen").size(14.0).color(t.text), |ui| {
        for (minutes, label) in SNOOZES {
            if ui.button(label).clicked() {
                picked = Some(minutes);
                ui.close();
            }
        }
    });
    picked
}

/// Draws the window when a reminder is due.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let now = now();
    let today = chrono::Local::now().date_naive();
    let due = app.engine.session.due_reminders(now);
    let follow = app.engine.session.due_follow_ups(today, now);
    let tasks = app.engine.session.due_task_reminders(today, now);
    if due.is_empty() && follow.is_empty() && tasks.is_empty() {
        app.announced_reminders.clear();
        return;
    }
    let mut keys: HashSet<(String, i64)> = due.iter().map(|a| (a.id.clone(), a.start)).collect();
    keys.extend(follow.iter().map(|(m, d)| (format!("followup:{}", m.id), i64::from(d.num_days_from_ce()))));
    keys.extend(tasks.iter().map(|t| (format!("task:{}", t.id), t.due.unwrap_or(0))));
    if keys.iter().any(|k| !app.announced_reminders.contains(k)) {
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
    }
    if app.view.notifications.reminders {
        let prefs = app.view.notifications.clone();
        let mut alerts = Vec::new();
        for a in due.iter().filter(|a| !app.announced_reminders.contains(&(a.id.clone(), a.start))) {
            let mut details = relative(now, a.start);
            if !a.location.is_empty() {
                details = format!("{details}, {}", a.location);
            }
            alerts.push(notifications::reminder_alert(&prefs, format!("appt:{}:{}", a.start, a.id), "Afspraak", &a.title, &details));
        }
        for (m, d) in &follow {
            if !app.announced_reminders.contains(&(format!("followup:{}", m.id), i64::from(d.num_days_from_ce()))) {
                let details = format!("van {}", m.from.display());
                alerts.push(notifications::reminder_alert(&prefs, format!("mail:{}", m.id), "Opvolgen", &m.subject, &details));
            }
        }
        for task in &tasks {
            if !app.announced_reminders.contains(&(format!("task:{}", task.id), task.due.unwrap_or(0))) {
                let day = follow_up_due(to_local(task.due.unwrap_or(now)).date(), today);
                alerts.push(notifications::reminder_alert(&prefs, format!("task:{}", task.id), "Taak", &task.title, &day));
            }
        }
        for alert in alerts {
            app.alert(ctx, alert, false);
        }
    }
    app.announced_reminders = keys;
    // The soonest reminder decides when to look again, so the window shows up on time.
    ctx.request_repaint_after(std::time::Duration::from_secs(5));
    let mut action: Option<Action> = None;
    let count = due.len() + follow.len() + tasks.len();
    let title = if count == 1 { "Herinnering".to_string() } else { format!("Herinneringen ({count})") };
    egui::Window::new(&title)
        .id(egui::Id::new("reminders"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::RIGHT_TOP, vec2(-16.0, 96.0))
        .frame(helder::window_frame(ctx, t))
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 8.0;
            let head = vec2(WIDTH, helder::BUTTON_HEIGHT);
            ui.allocate_ui_with_layout(head, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(helder::Icon::Bell.image(helder::ICON, t.accent_text));
                ui.label(RichText::new(&title).size(16.0).strong().color(t.text));
                if count > 1 {
                    helder::right(ui, |ui| {
                        if helder::text_button(ui, t, "Alles sluiten").clicked() {
                            action = Some(Action::DismissAll);
                        }
                    });
                }
            });
            for a in &due {
                let mut details = vec![(helder::Icon::Clock, when(a.start, a.all_day))];
                if !a.location.is_empty() {
                    details.push((helder::Icon::MapPin, a.location.clone()));
                }
                let lead = (relative(now, a.start), t.accent_text);
                card(ui, t, helder::Icon::Calendar, "Afspraak", &a.title, lead, &details, |ui| {
                    if helder::primary_button(ui, t, helder::Icon::Calendar, "Openen").clicked() {
                        action = Some(Action::Open(a.id.clone(), a.start));
                    }
                    if helder::tool_button(ui, t, helder::Icon::X, "Sluiten").clicked() {
                        action = Some(Action::Dismiss(a.id.clone(), a.start));
                    }
                    if let Some(minutes) = snooze(ui, t) {
                        action = Some(Action::Snooze(a.id.clone(), a.start, minutes));
                    }
                });
            }
            // Flagged mail whose day has come, as Outlook reminds of flagged items.
            for (m, day) in &follow {
                let late = *day < today;
                let lead = (follow_up_due(*day, today), if late { t.flag } else { t.accent_text });
                let details = [(helder::Icon::User, format!("van {}", m.from.display()))];
                card(ui, t, helder::Icon::Flag, "Opvolgen", &m.subject, lead, &details, |ui| {
                    if helder::primary_button(ui, t, helder::Icon::Mail, "Openen").clicked() {
                        action = Some(Action::OpenMail(m.id.clone()));
                    }
                    if helder::tool_button(ui, t, helder::Icon::CircleCheck, "Voltooid").on_hover_text("Haalt de vlag en de datum weg.").clicked() {
                        action = Some(Action::Done(m.id.clone()));
                    }
                    if helder::tool_button(ui, t, helder::Icon::X, "Sluiten")
                        .on_hover_text("Sluit de herinnering; de vlag en de datum blijven.")
                        .clicked()
                    {
                        action = Some(Action::DismissMail(m.id.clone()));
                    }
                    if let Some(minutes) = snooze(ui, t) {
                        action = Some(Action::SnoozeMail(m.id.clone(), minutes));
                    }
                });
            }
            // Tasks whose due day has come, as Outlook reminds of tasks with a due date.
            for task in &tasks {
                let day = to_local(task.due.unwrap_or(now)).date();
                let late = day < today;
                let lead = (follow_up_due(day, today), if late { t.flag } else { t.accent_text });
                card(ui, t, helder::Icon::SquareCheck, "Taak", &task.title, lead, &[], |ui| {
                    if helder::primary_button(ui, t, helder::Icon::SquareCheck, "Openen").clicked() {
                        action = Some(Action::OpenTask(task.id.clone()));
                    }
                    if helder::tool_button(ui, t, helder::Icon::CircleCheck, "Voltooid").on_hover_text("Vinkt de taak af.").clicked() {
                        action = Some(Action::DoneTask(task.id.clone()));
                    }
                    if helder::tool_button(ui, t, helder::Icon::X, "Sluiten").on_hover_text("Sluit de herinnering; de taak blijft open.").clicked() {
                        action = Some(Action::DismissTask(task.id.clone()));
                    }
                    if let Some(minutes) = snooze(ui, t) {
                        action = Some(Action::SnoozeTask(task.id.clone(), minutes));
                    }
                });
            }
        });
    match action {
        Some(Action::Open(id, start)) => {
            app.module = Module::Calendar;
            app.calendar.focus = to_local(start).date();
            app.open_appointment(&id, start);
        }
        Some(Action::Dismiss(id, start)) => app.run("reminder.dismiss", json!({ "id": id, "start": start })),
        Some(Action::Snooze(id, start, minutes)) => app.run("reminder.snooze", json!({ "id": id, "start": start, "minutes": minutes })),
        Some(Action::OpenMail(id)) => app.open_message(&id),
        Some(Action::Done(id)) => app.run("message.flag", json!({ "id": id })),
        Some(Action::DismissMail(id)) => app.run("followup.dismiss", json!({ "id": id })),
        Some(Action::SnoozeMail(id, minutes)) => app.run("followup.snooze", json!({ "id": id, "minutes": minutes })),
        Some(Action::OpenTask(id)) => {
            app.module = Module::Tasks;
            app.run("task.select", json!({ "id": id }));
        }
        Some(Action::DoneTask(id)) => app.run("task.complete", json!({ "id": id, "done": true })),
        Some(Action::DismissTask(id)) => app.run("task.reminder_dismiss", json!({ "id": id })),
        Some(Action::SnoozeTask(id, minutes)) => app.run("task.reminder_snooze", json!({ "id": id, "minutes": minutes })),
        Some(Action::DismissAll) => {
            for a in &due {
                app.run("reminder.dismiss", json!({ "id": a.id, "start": a.start }));
            }
            for (m, _) in &follow {
                app.run("followup.dismiss", json!({ "id": m.id }));
            }
            for task in &tasks {
                app.run("task.reminder_dismiss", json!({ "id": task.id }));
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times_read_naturally() {
        assert_eq!(relative(1000, 1030), "Begint nu");
        assert_eq!(relative(1000, 1000 + 60), "Begint over 1 minuut");
        assert_eq!(relative(1000, 1000 + 15 * 60), "Begint over 15 minuten");
        assert_eq!(relative(1000, 1000 + 2 * 3600), "Begint over 2 uur");
        assert_eq!(relative(1000, 1000 + 3 * 86_400), "Begint over 3 dagen");
        assert_eq!(relative(1000, 1000 - 5 * 60), "Begon 5 minuten geleden");
        assert_eq!(relative(1000 + 86_400, 1000), "Begon 1 dag geleden");
    }

    #[test]
    fn a_follow_up_day_reads_as_today_yesterday_or_lapsed() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        assert_eq!(follow_up_due(today, today), "Vandaag");
        assert_eq!(follow_up_due(today.pred_opt().unwrap(), today), "Gisteren");
        assert_eq!(follow_up_due(chrono::NaiveDate::from_ymd_opt(2026, 10, 16).unwrap(), today), "Verlopen op vrijdag 16 oktober");
    }
}
