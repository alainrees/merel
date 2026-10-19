//! The tasks module: a list with an open/done filter, and the form to add or change a task.

use chrono::{Days, NaiveDate, NaiveTime};
use egui::{Align2, Color32, CornerRadius, FontId, Margin, RichText, Sense, Stroke, Ui, vec2};
use serde_json::json;

use mailcraft_engine::{CommandError, tasks::TaskFilter};
use mailcraft_model::{Priority, Task};

use crate::calendar::{to_local, to_timestamp, today};
use crate::{MailApp, Tokens};

const DATE_FORMAT: &str = "%d-%m-%Y";

/// What the tasks module remembers between frames.
pub struct State {
    pub filter: TaskFilter,
    /// What the search box holds; empty shows every task the filter allows.
    pub query: String,
    pub editor: Option<Editor>,
}

/// The search box's widget id, so Ctrl+E can put the cursor there.
pub(crate) const SEARCH_BOX: &str = "tasks_search_box";

impl Default for State {
    fn default() -> Self {
        State { filter: TaskFilter::Open, query: String::new(), editor: None }
    }
}

/// The form's fields as text.
#[derive(Default)]
pub struct Editor {
    pub id: Option<String>,
    /// Where a new task goes: an account id, or empty for this computer.
    pub account_id: String,
    pub title: String,
    pub due: String,
    pub priority: Priority,
    pub notes: String,
    pub error: Option<String>,
}

impl Editor {
    pub fn from_task(t: &Task) -> Self {
        Editor {
            id: Some(t.id.clone()),
            account_id: t.account_id.clone().unwrap_or_default(),
            title: t.title.clone(),
            due: t.due.map(|d| to_local(d).format(DATE_FORMAT).to_string()).unwrap_or_default(),
            priority: t.priority,
            notes: t.notes.clone(),
            error: None,
        }
    }

    /// The `task.save` parameters, or what is wrong with the fields.
    pub fn params(&self) -> Result<serde_json::Value, String> {
        if self.title.trim().is_empty() {
            return Err("Vul een titel in.".into());
        }
        let due = if self.due.trim().is_empty() {
            None
        } else {
            let date = NaiveDate::parse_from_str(self.due.trim(), DATE_FORMAT)
                .map_err(|_| "Vul een geldige vervaldatum in, bijvoorbeeld 18-10-2026.".to_string())?;
            Some(to_timestamp(date.and_time(NaiveTime::MIN)).ok_or("Deze datum bestaat niet in deze tijdzone.")?)
        };
        let priority = match self.priority {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
        };
        let mut p = json!({ "title": self.title, "notes": self.notes, "due": due, "priority": priority });
        if let Some(id) = &self.id {
            p["id"] = json!(id);
        }
        if !self.account_id.is_empty() {
            p["account_id"] = json!(self.account_id);
        }
        Ok(p)
    }
}

impl MailApp {
    pub fn new_task(&mut self) {
        self.tasks.editor = Some(Editor::default());
    }

    pub fn edit_selected_task(&mut self) {
        if let Some(t) = self.engine.session.selected_task() {
            self.tasks.editor = Some(Editor::from_task(t));
        }
    }
}

fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("title") => "Vul een titel in.".into(),
        CommandError::Store(m) => format!("Opslaan is mislukt: {m}"),
        CommandError::NotFound => "Deze taak bestaat niet meer.".into(),
        e => e.to_string(),
    }
}

/// "vandaag", "morgen", "gisteren" or the date.
fn due_label(due: i64) -> String {
    let day = to_local(due).date();
    let today = today();
    if day == today {
        "vandaag".into()
    } else if Some(day) == today.checked_add_days(Days::new(1)) {
        "morgen".into()
    } else if Some(day) == today.checked_sub_days(Days::new(1)) {
        "gisteren".into()
    } else {
        day.format("%d-%m-%Y").to_string()
    }
}

fn priority_label(p: Priority) -> &'static str {
    match p {
        Priority::Low => "Laag",
        Priority::Normal => "Normaal",
        Priority::High => "Hoog",
    }
}

fn list_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("tasks-list")
        .default_size(360.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(Margin::symmetric(10, 10)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 28.0).max(60.0);
                ui.add(
                    egui::TextEdit::singleline(&mut app.tasks.query).id(egui::Id::new(SEARCH_BOX)).hint_text("Zoeken in taken").desired_width(width),
                );
                if !app.tasks.query.is_empty() && ui.small_button("✕").on_hover_text("Zoekopdracht wissen").clicked() {
                    app.tasks.query.clear();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for (label, f) in [("Open", TaskFilter::Open), ("Voltooid", TaskFilter::Done), ("Alles", TaskFilter::All)] {
                    if ui.selectable_label(app.tasks.filter == f, label).clicked() {
                        app.tasks.filter = f;
                    }
                }
            });
            ui.add_space(8.0);
            let selected = app.engine.session.selected_task.clone();
            let list: Vec<Task> = app.engine.session.tasks_matching(app.tasks.filter, &app.tasks.query).into_iter().cloned().collect();
            if list.is_empty() {
                let text = if !app.tasks.query.trim().is_empty() {
                    "Geen taak gevonden."
                } else if app.tasks.filter == TaskFilter::Done {
                    "Nog niets voltooid."
                } else {
                    "Geen taken. Lekker rustig."
                };
                ui.label(RichText::new(text).color(t.text_dim));
            }
            let midnight = today().and_time(NaiveTime::MIN);
            let today_ts = to_timestamp(midnight).unwrap_or(0);
            let mut pick = None;
            let mut toggle = None;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for task in &list {
                    let on = selected.as_deref() == Some(task.id.as_str());
                    let r = ui.add_sized(
                        [ui.available_width(), 40.0],
                        egui::Button::new("").fill(if on { t.checked } else { Color32::TRANSPARENT }).frame_when_inactive(on),
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &task.title));
                    let rect = r.rect.shrink2(vec2(8.0, 4.0));
                    // The checkbox is its own widget, drawn over the row.
                    let check = egui::Rect::from_center_size(egui::pos2(rect.left() + 10.0, rect.center().y), vec2(18.0, 18.0));
                    let c = ui.interact(check, ui.make_persistent_id(("task-check", &task.id)), Sense::click());
                    c.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, true, format!("{} voltooid", task.title)));
                    let p = ui.painter();
                    p.rect_stroke(
                        check,
                        CornerRadius::same(3),
                        Stroke::new(1.5, if task.done { t.accent } else { t.text_dim }),
                        egui::StrokeKind::Inside,
                    );
                    if task.done {
                        p.text(check.center(), Align2::CENTER_CENTER, "✔", FontId::proportional(13.0), t.accent);
                    }
                    let x = rect.left() + 28.0;
                    let title_color = if task.done { t.text_dim } else { t.text };
                    p.text(egui::pos2(x, rect.center().y - 1.0), Align2::LEFT_BOTTOM, &task.title, FontId::proportional(13.5), title_color);
                    let mut sub = Vec::new();
                    if let Some(due) = task.due {
                        sub.push(due_label(due));
                    }
                    if task.priority == Priority::High {
                        sub.push("hoge prioriteit".into());
                    }
                    let sub_color = if task.overdue(today_ts) { t.flag } else { t.text_dim };
                    p.text(egui::pos2(x, rect.center().y + 2.0), Align2::LEFT_TOP, sub.join(" · "), FontId::proportional(11.5), sub_color);
                    if c.clicked() {
                        toggle = Some(task.id.clone());
                    } else if r.clicked() {
                        pick = Some(task.id.clone());
                    }
                }
            });
            if let Some(id) = toggle {
                app.run("task.complete", json!({ "id": id }));
            }
            if let Some(id) = pick {
                app.run("task.select", json!({ "id": id }));
                app.tasks.editor = None;
            }
        });
}

fn detail(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(task) = app.engine.session.selected_task().cloned() else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Kies een taak, of maak een nieuwe taak.").color(t.text_dim).size(16.0));
        });
        return;
    };
    ui.label(RichText::new(&task.title).size(22.0).strong().color(if task.done { t.text_dim } else { t.text }));
    ui.add_space(8.0);
    let row = |ui: &mut Ui, label: &str, value: String| {
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new(label).color(t.text_dim)));
            ui.label(RichText::new(value).color(t.text));
        });
    };
    row(ui, "Status", if task.done { "Voltooid".into() } else { "Open".into() });
    row(ui, "Vervaldatum", task.due.map(due_label).unwrap_or_else(|| "Geen".into()));
    row(ui, "Prioriteit", priority_label(task.priority).into());
    if !app.engine.session.item_accounts.is_empty() {
        row(ui, "Opgeslagen in", crate::contacts::store_name(&app.engine.session, task.account_id.as_deref()));
    }
    if !task.notes.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Notities").color(t.text_dim));
        ui.label(RichText::new(&task.notes).color(t.text));
    }
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new("Bewerken").min_size(vec2(90.0, 26.0))).clicked() {
            app.edit_selected_task();
        }
        let flip = if task.done { "Heropenen" } else { "Voltooien" };
        if ui.add(egui::Button::new(flip).min_size(vec2(90.0, 26.0))).clicked() {
            app.run("task.complete", serde_json::Value::Null);
        }
    });
}

fn form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let session = &app.engine.session;
    let Some(editor) = app.tasks.editor.as_mut() else { return };
    ui.label(RichText::new(if editor.id.is_some() { "Taak bewerken" } else { "Nieuwe taak" }).size(18.0).strong().color(t.text));
    ui.add_space(8.0);
    if editor.id.is_none() {
        crate::contacts::store_picker(ui, t, session, 110.0, &mut editor.account_id);
    }
    let field = |ui: &mut Ui, label: &str, value: &mut String, width: f32| {
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 22.0], egui::Label::new(RichText::new(label).color(t.text)));
            ui.add(egui::TextEdit::singleline(value).desired_width(width));
        });
    };
    field(ui, "Titel", &mut editor.title, 320.0);
    field(ui, "Vervaldatum", &mut editor.due, 110.0);
    ui.horizontal(|ui| {
        ui.add_sized([110.0, 22.0], egui::Label::new(RichText::new("Prioriteit").color(t.text)));
        for p in [Priority::Low, Priority::Normal, Priority::High] {
            ui.radio_value(&mut editor.priority, p, priority_label(p));
        }
    });
    ui.horizontal(|ui| {
        ui.add_sized([110.0, 22.0], egui::Label::new(RichText::new("Notities").color(t.text)));
        ui.add(egui::TextEdit::multiline(&mut editor.notes).desired_width(320.0).desired_rows(4));
    });
    ui.label(RichText::new("Vervaldatum als dag-maand-jaar, of leeg voor geen deadline.").size(11.5).color(t.text_dim));
    if let Some(e) = &editor.error {
        ui.label(RichText::new(e).color(t.flag));
    }
    ui.add_space(8.0);
    let mut save = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        save = ui.add(egui::Button::new(RichText::new("Opslaan").color(t.on_accent)).fill(t.accent).min_size(vec2(90.0, 26.0))).clicked();
        cancel = ui.add(egui::Button::new("Annuleren").min_size(vec2(90.0, 26.0))).clicked();
    });
    if save {
        match editor.params() {
            Ok(params) => match app.engine.run("task.save", &params) {
                Ok(()) => cancel = true,
                Err(e) => editor.error = Some(describe(&e)),
            },
            Err(e) => editor.error = Some(e),
        }
    }
    if cancel {
        app.tasks.editor = None;
    }
}

/// The tasks module's panes; the ribbon is drawn by the caller.
pub fn show(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    list_pane(app, ui, t);
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading).inner_margin(Margin::same(24))).show(ui, |ui| {
        if app.tasks.editor.is_some() {
            form(app, ui, t);
        } else {
            detail(app, ui, t);
        }
    });
}

/// The ribbon for the tasks module.
pub fn ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let selected = app.engine.session.selected_task().cloned();
    let has = selected.is_some();
    if crate::ribbon_action(ui, t, "✔", "Nieuwe taak", true) {
        app.new_task();
    }
    ui.separator();
    if crate::ribbon_action(ui, t, "✎", "Bewerken", has) {
        app.edit_selected_task();
    }
    let flip = if selected.as_ref().is_some_and(|t| t.done) { "Heropenen" } else { "Voltooien" };
    if crate::ribbon_action(ui, t, "☑", flip, has) {
        app.run("task.complete", serde_json::Value::Null);
    }
    ui.separator();
    if crate::ribbon_action(ui, t, "🗑", "Verwijderen", has) {
        app.run("task.delete", serde_json::Value::Null);
        app.tasks.editor = None;
    }
    if crate::ribbon_action(ui, t, "⟳", "Vernieuwen", !app.engine.session.item_accounts.is_empty()) {
        app.run("sync.refresh", serde_json::Value::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_params_parse_the_due_day() {
        let mut e = Editor { title: "Offerte".into(), due: "21-10-2026".into(), priority: Priority::High, ..Editor::default() };
        let p = e.params().unwrap();
        assert_eq!(p["priority"], "high");
        assert_eq!(to_local(p["due"].as_i64().unwrap()).date(), NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        e.due = "  ".into();
        assert!(e.params().unwrap()["due"].is_null());
        e.due = "21/10/2026".into();
        assert!(e.params().unwrap_err().contains("vervaldatum"));
        e.title = String::new();
        assert_eq!(e.params(), Err("Vul een titel in.".to_string()));
    }
}
