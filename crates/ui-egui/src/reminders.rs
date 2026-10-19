//! The reminders window: the appointments whose reminder is due, the flagged messages whose
//! follow-up day has come and the tasks whose due day has come, each to open, close or put off. It appears on its own, asks for the
//! window's attention once per reminder, and goes away when nothing is due.

use std::collections::HashSet;

use chrono::Datelike;
use egui::{Align2, RichText, vec2};
use serde_json::json;

use crate::calendar::{MONTHS, WEEKDAYS, to_local};
use crate::{MailApp, Module, Tokens};

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
    app.announced_reminders = keys;
    // The soonest reminder decides when to look again, so the window shows up on time.
    ctx.request_repaint_after(std::time::Duration::from_secs(5));
    let mut action: Option<Action> = None;
    let count = due.len() + follow.len() + tasks.len();
    let title = if count == 1 { "Herinnering".to_string() } else { format!("Herinneringen ({count})") };
    egui::Window::new(title)
        .id(egui::Id::new("reminders"))
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .anchor(Align2::RIGHT_TOP, vec2(-16.0, 96.0))
        .show(ctx, |ui| {
            for a in &due {
                egui::Frame::new().fill(t.list).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.label(RichText::new(&a.title).strong().color(t.text));
                    ui.label(RichText::new(relative(now, a.start)).color(t.accent_text));
                    ui.label(RichText::new(when(a.start, a.all_day)).size(12.0).color(t.text_dim));
                    if !a.location.is_empty() {
                        ui.label(RichText::new(&a.location).size(12.0).color(t.text_dim));
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Openen").clicked() {
                            action = Some(Action::Open(a.id.clone(), a.start));
                        }
                        if ui.button("Sluiten").clicked() {
                            action = Some(Action::Dismiss(a.id.clone(), a.start));
                        }
                        egui::ComboBox::from_id_salt(("snooze", &a.id, a.start)).selected_text("Uitstellen…").width(130.0).show_ui(ui, |ui| {
                            for (minutes, label) in SNOOZES {
                                if ui.selectable_label(false, label).clicked() {
                                    action = Some(Action::Snooze(a.id.clone(), a.start, minutes));
                                }
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
            // Flagged mail whose day has come, as Outlook reminds of flagged items.
            for (m, day) in &follow {
                egui::Frame::new().fill(t.list).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.label(RichText::new(format!("⚑ Opvolgen: {}", m.subject)).strong().color(t.text));
                    let late = *day < today;
                    ui.label(RichText::new(follow_up_due(*day, today)).color(if late { t.flag } else { t.accent_text }));
                    ui.label(RichText::new(format!("van {}", m.from.display())).size(12.0).color(t.text_dim));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Openen").clicked() {
                            action = Some(Action::OpenMail(m.id.clone()));
                        }
                        if ui.button("Voltooid").on_hover_text("Haalt de vlag en de datum weg.").clicked() {
                            action = Some(Action::Done(m.id.clone()));
                        }
                        if ui.button("Sluiten").on_hover_text("Sluit de herinnering; de vlag en de datum blijven.").clicked() {
                            action = Some(Action::DismissMail(m.id.clone()));
                        }
                        egui::ComboBox::from_id_salt(("snooze-followup", &m.id)).selected_text("Uitstellen…").width(130.0).show_ui(ui, |ui| {
                            for (minutes, label) in SNOOZES {
                                if ui.selectable_label(false, label).clicked() {
                                    action = Some(Action::SnoozeMail(m.id.clone(), minutes));
                                }
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
            // Tasks whose due day has come, as Outlook reminds of tasks with a due date.
            for task in &tasks {
                egui::Frame::new().fill(t.list).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.label(RichText::new(format!("☑ Taak: {}", task.title)).strong().color(t.text));
                    let day = to_local(task.due.unwrap_or(now)).date();
                    let late = day < today;
                    ui.label(RichText::new(follow_up_due(day, today)).color(if late { t.flag } else { t.accent_text }));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Openen").clicked() {
                            action = Some(Action::OpenTask(task.id.clone()));
                        }
                        if ui.button("Voltooid").on_hover_text("Vinkt de taak af.").clicked() {
                            action = Some(Action::DoneTask(task.id.clone()));
                        }
                        if ui.button("Sluiten").on_hover_text("Sluit de herinnering; de taak blijft open.").clicked() {
                            action = Some(Action::DismissTask(task.id.clone()));
                        }
                        egui::ComboBox::from_id_salt(("snooze-task", &task.id)).selected_text("Uitstellen…").width(130.0).show_ui(ui, |ui| {
                            for (minutes, label) in SNOOZES {
                                if ui.selectable_label(false, label).clicked() {
                                    action = Some(Action::SnoozeTask(task.id.clone(), minutes));
                                }
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
            if count > 1 && ui.button("Alles sluiten").clicked() {
                action = Some(Action::DismissAll);
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
