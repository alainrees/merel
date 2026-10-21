//! The tasks module: a list with an open/done filter, the flagged mail under it (Outlook's
//! To-Do list shows those too), and the form to add or change a task.

use chrono::{Days, NaiveDate, NaiveTime};
use egui::{Align2, Color32, CornerRadius, FontId, Margin, RichText, Sense, Stroke, Ui, vec2};
use serde_json::json;

use merel_engine::{CommandError, followup, tasks::TaskFilter};
use merel_model::{Frequency, MessageHeader, Priority, Repeat, Task};

use crate::calendar::{to_local, to_timestamp, today};
use crate::helder::{self, Icon};
use crate::{MailApp, Tokens};

const DATE_FORMAT: &str = "%d-%m-%Y";
/// The calendar button beside the due date.
pub const DUE_PICKER: &str = "Vervaldatum kiezen";

/// Before the title of a repeating task in the list, as the calendar marks a series.
const REPEAT_MARK: &str = "🔁";

/// What the tasks module remembers between frames.
pub struct State {
    pub filter: TaskFilter,
    /// What the search box holds; empty shows every task the filter allows.
    pub query: String,
    pub editor: Option<Editor>,
}

/// The search box's widget id, so Ctrl+E can put the cursor there.
pub(crate) const SEARCH_BOX: &str = "tasks_search_box";

/// The heading over the flagged messages in the list, as Outlook's To-Do list has it.
pub(crate) const FLAGGED_HEADING: &str = "Gemarkeerde e-mail";

/// The flagged messages the list shows under the tasks: those matching `query` (subject or
/// sender), the ones with a due day first and soonest first, then the newest mail first.
fn flagged_mail(app: &MailApp, query: &str) -> Vec<(MessageHeader, Option<NaiveDate>)> {
    let needle = query.trim().to_lowercase();
    let mut list: Vec<(MessageHeader, Option<NaiveDate>)> = app
        .engine
        .session
        .messages
        .iter()
        .filter(|m| m.flags.flagged)
        .filter(|m| needle.is_empty() || m.subject.to_lowercase().contains(&needle) || m.from.display().to_lowercase().contains(&needle))
        .map(|m| (m.clone(), app.engine.session.follow_up(&m.id).and_then(followup::due_day)))
        .collect();
    list.sort_by(|(a, da), (b, db)| match (da, db) {
        (Some(x), Some(y)) => x.cmp(y).then(b.date.cmp(&a.date)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.date.cmp(&a.date),
    });
    list
}

/// "vandaag", "morgen", "verlopen op …" or "vóór …" for a follow-up day.
fn follow_up_label(due: NaiveDate, today: NaiveDate) -> String {
    if due == today {
        "vandaag".into()
    } else if due < today {
        format!("verlopen op {}", crate::dates::day_name(due))
    } else if Some(due) == today.checked_add_days(Days::new(1)) {
        "morgen".into()
    } else {
        format!("vóór {}", crate::dates::day_name(due))
    }
}

impl Default for State {
    fn default() -> Self {
        State { filter: TaskFilter::Open, query: String::new(), editor: None }
    }
}

/// The form's fields as text.
pub struct Editor {
    pub id: Option<String>,
    /// Where a new task goes: an account id, or empty for this computer.
    pub account_id: String,
    pub title: String,
    pub due: String,
    pub priority: Priority,
    pub notes: String,
    /// Outlook's "Terugkeerpatroon": how often the task comes back, if at all.
    pub repeat: Option<Frequency>,
    /// Every so many units; 0 counts as 1.
    pub interval: u32,
    /// The formatting of the notes, with the bar over them as in the message window.
    pub notes_editor: crate::rich_edit::Editor,
    pub error: Option<String>,
}

/// The notes editor's salt: a task's form shows one at a time.
const NOTES: &str = "task-notes";

impl Default for Editor {
    fn default() -> Self {
        Editor {
            id: None,
            account_id: String::new(),
            title: String::new(),
            due: String::new(),
            priority: Priority::default(),
            notes: String::new(),
            repeat: None,
            interval: 0,
            notes_editor: crate::rich_edit::Editor::new(NOTES, "", None),
            error: None,
        }
    }
}

/// The repeat choices of the form, with their names.
pub const REPEATS: [(Option<Frequency>, &str); 5] = [
    (None, "Niet"),
    (Some(Frequency::Daily), "Dagelijks"),
    (Some(Frequency::Weekly), "Wekelijks"),
    (Some(Frequency::Monthly), "Maandelijks"),
    (Some(Frequency::Yearly), "Jaarlijks"),
];

pub const REPEAT_NEEDS_DUE: &str = "Een herhalende taak heeft een vervaldatum nodig en blijft op deze computer.";

impl Editor {
    pub fn from_task(t: &Task) -> Self {
        Editor {
            id: Some(t.id.clone()),
            account_id: t.account_id.clone().unwrap_or_default(),
            title: t.title.clone(),
            due: t.due.map(|d| to_local(d).format(DATE_FORMAT).to_string()).unwrap_or_default(),
            priority: t.priority,
            notes: t.notes.clone(),
            repeat: t.repeat.map(|r| r.frequency),
            interval: t.repeat.map(|r| r.interval).unwrap_or(1),
            notes_editor: crate::rich_edit::Editor::new(NOTES, &t.notes, t.notes_rich.clone()),
            error: None,
        }
    }

    /// The repeat the fields describe.
    pub fn repeat(&self) -> Option<Repeat> {
        self.repeat.map(|frequency| Repeat { frequency, interval: self.interval.max(1) })
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
        if self.repeat.is_some() && (due.is_none() || !self.account_id.is_empty()) {
            return Err(REPEAT_NEEDS_DUE.into());
        }
        let mut p = json!({ "title": self.title, "notes": self.notes, "due": due, "priority": priority, "repeat": self.repeat(), "notes_rich": self.notes_editor.stored(&self.notes) });
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
        CommandError::Param("repeat") => REPEAT_NEEDS_DUE.into(),
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
        .show_separator_line(false)
        .frame(crate::helder::card_frame(t).inner_margin(Margin::same(12)).outer_margin(Margin { left: 0, right: 6, top: 0, bottom: 12 }))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 40.0).max(60.0);
                crate::helder::search_field(ui, t, &mut app.tasks.query, egui::Id::new(SEARCH_BOX), "Zoeken in taken", width);
                if !app.tasks.query.is_empty()
                    && crate::helder::quiet(ui, t, |ui| crate::helder::icon_button(ui, t, crate::helder::Icon::X, crate::SEARCH_CLEAR)).clicked()
                {
                    app.tasks.query.clear();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for (label, f) in [("Open", TaskFilter::Open), ("Voltooid", TaskFilter::Done), ("Alles", TaskFilter::All)] {
                    if crate::helder::chip(ui, t, label, app.tasks.filter == f).clicked() {
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
            let mut open_mail = None;
            let mut unflag = None;
            // Flagged mail belongs with the open work, as in Outlook's To-Do list; it is not a
            // task, so the done filter leaves it out.
            let flagged = if app.tasks.filter == TaskFilter::Done { Vec::new() } else { flagged_mail(app, &app.tasks.query) };
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
                    let title = if task.repeat.is_some() { format!("{REPEAT_MARK} {}", task.title) } else { task.title.clone() };
                    p.text(egui::pos2(x, rect.center().y - 1.0), Align2::LEFT_BOTTOM, title, FontId::proportional(13.5), title_color);
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
                if !flagged.is_empty() {
                    ui.add_space(12.0);
                    ui.label(RichText::new(FLAGGED_HEADING).strong().color(t.text));
                    ui.add_space(4.0);
                    let today = today();
                    for (m, due) in &flagged {
                        let r =
                            ui.add_sized([ui.available_width(), 40.0], egui::Button::new("").fill(Color32::TRANSPARENT).frame_when_inactive(false));
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &m.subject));
                        let r = r.on_hover_text("Opent het bericht in E-mail.");
                        let rect = r.rect.shrink2(vec2(8.0, 4.0));
                        // The flag is its own widget: a click takes it off, as the checkbox finishes a task.
                        let check = egui::Rect::from_center_size(egui::pos2(rect.left() + 10.0, rect.center().y), vec2(18.0, 18.0));
                        let c = ui.interact(check, ui.make_persistent_id(("mail-flag", &m.id)), Sense::click());
                        c.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, true, format!("Vlag weghalen van {}", m.subject)));
                        let c = c.on_hover_text("Haalt de vlag (en de datum) weg.");
                        let p = ui.painter();
                        p.text(check.center(), Align2::CENTER_CENTER, "⚑", FontId::proportional(15.0), t.flag);
                        let x = rect.left() + 28.0;
                        p.text(egui::pos2(x, rect.center().y - 1.0), Align2::LEFT_BOTTOM, &m.subject, FontId::proportional(13.5), t.text);
                        let mut sub = vec![format!("van {}", m.from.display())];
                        if let Some(d) = due {
                            sub.push(follow_up_label(*d, today));
                        }
                        let late = due.is_some_and(|d| d < today);
                        p.text(
                            egui::pos2(x, rect.center().y + 2.0),
                            Align2::LEFT_TOP,
                            sub.join(" · "),
                            FontId::proportional(11.5),
                            if late { t.flag } else { t.text_dim },
                        );
                        if c.clicked() {
                            unflag = Some(m.id.clone());
                        } else if r.clicked() {
                            open_mail = Some(m.id.clone());
                        }
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
            if let Some(id) = unflag {
                app.run("message.flag", json!({ "id": id }));
            }
            if let Some(id) = open_mail {
                app.open_message(&id);
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
    helder::quiet(ui, t, |ui| {
        ui.horizontal(|ui| {
            if helder::tool_button(ui, t, Icon::Pencil, "Bewerken").clicked() {
                app.edit_selected_task();
            }
            let flip = if task.done { "Heropenen" } else { "Voltooien" };
            if helder::tool_button(ui, t, Icon::SquareCheck, flip).clicked() {
                app.run("task.complete", serde_json::Value::Null);
            }
        });
    });
    ui.add_space(12.0);
    ui.label(RichText::new(&task.title).size(22.0).strong().color(if task.done { t.text_dim } else { t.text }));
    ui.add_space(4.0);
    let row = |ui: &mut Ui, icon: Icon, label: &str, value: String| {
        helder::icon_row(ui, t, icon, |ui| ui.label(RichText::new(value).size(14.0).color(t.text))).on_hover_text(label);
    };
    row(ui, Icon::CircleCheck, "Status", if task.done { "Voltooid".into() } else { "Open".into() });
    row(ui, Icon::Calendar, "Vervaldatum", task.due.map(due_label).unwrap_or_else(|| "Geen vervaldatum".into()));
    row(ui, Icon::Flag, "Prioriteit", format!("Prioriteit {}", priority_label(task.priority).to_lowercase()));
    if let Some(r) = task.repeat {
        row(ui, Icon::Repeat, "Herhalen", r.describe());
    }
    if !app.engine.session.item_accounts.is_empty() {
        row(ui, Icon::Folder, "Opgeslagen in", crate::contacts::store_name(&app.engine.session, task.account_id.as_deref()));
    }
    if !task.notes.is_empty() {
        helder::rule(ui, t);
        helder::icon_row(ui, t, Icon::Notes, |ui| ui.add(egui::Label::new(RichText::new(&task.notes).size(14.0).color(t.text)).wrap()));
    }
}

fn form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let session = &app.engine.session;
    let Some(editor) = app.tasks.editor.as_mut() else { return };
    let mut save = false;
    let mut cancel = false;
    helder::quiet(ui, t, |ui| {
        ui.horizontal(|ui| {
            save = helder::primary_button(ui, t, Icon::Save, "Opslaan").clicked();
            helder::bar_separator(ui, t);
            cancel = helder::tool_button(ui, t, Icon::X, "Annuleren").clicked();
        });
    });
    ui.add_space(12.0);
    let width = ui.available_width().min(620.0) - 44.0;
    ui.horizontal(|ui| {
        helder::indent(ui);
        ui.label(RichText::new(if editor.id.is_some() { "Taak bewerken" } else { "Nieuwe taak" }).size(13.0).color(t.text_dim));
    });
    helder::icon_row(ui, t, Icon::Type, |ui| helder::underline_field(ui, t, &mut editor.title, "Titel toevoegen", width, true));
    if editor.id.is_none() {
        crate::contacts::store_picker(ui, t, session, &mut editor.account_id);
        if !editor.account_id.is_empty() {
            // A repeat is kept on this computer only: a server would not know it.
            editor.repeat = None;
        }
    }
    helder::icon_row(ui, t, Icon::Calendar, |ui| {
        helder::underline_field(ui, t, &mut editor.due, "Vervaldatum (dd-mm-jjjj)", 200.0, false)
            .on_hover_text("Vervaldatum als dag-maand-jaar, of leeg voor geen deadline.");
        crate::date_picker::button(ui, t, &mut editor.due, DUE_PICKER);
    });
    helder::icon_row(ui, t, Icon::Flag, |ui| {
        ui.label(RichText::new("Prioriteit").size(14.0).color(t.text));
        ui.add_space(4.0);
        for p in [Priority::Low, Priority::Normal, Priority::High] {
            if helder::chip(ui, t, priority_label(p), editor.priority == p).clicked() {
                editor.priority = p;
            }
        }
    });
    if editor.account_id.is_empty() {
        let name = REPEATS.iter().find(|(f, _)| *f == editor.repeat).map(|(_, n)| *n).unwrap_or("Niet");
        let shown = if editor.repeat.is_none() { "Niet herhalen" } else { name };
        helder::icon_row(ui, t, Icon::Repeat, |ui| {
            helder::select(ui, t, shown, 160.0, |ui| {
                for (f, name) in REPEATS {
                    if ui.selectable_label(editor.repeat == f, if f.is_none() { "Niet herhalen" } else { name }).clicked() {
                        editor.repeat = f;
                        ui.close();
                    }
                }
            });
            if let Some(f) = editor.repeat {
                let unit = match f {
                    Frequency::Daily => "dagen",
                    Frequency::Weekly => "weken",
                    Frequency::Monthly => "maanden",
                    Frequency::Yearly => "jaar",
                };
                ui.add_space(8.0);
                ui.label(RichText::new("om de").size(14.0).color(t.text));
                let mut text = editor.interval.max(1).to_string();
                if helder::number_field(ui, t, &mut text, "1").changed()
                    && let Ok(n) = text.trim().parse::<u32>()
                {
                    editor.interval = n.clamp(1, 99);
                }
                ui.label(RichText::new(unit).size(14.0).color(t.text));
            }
        });
        if editor.repeat.is_some() {
            helder::note(ui, t, "Na Voltooien komt de taak terug met de volgende vervaldatum.");
        }
    }
    ui.add_space(4.0);
    helder::notes_box(ui, t, &mut editor.notes_editor, &mut editor.notes, width, 100.0);
    if let Some(e) = &editor.error {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            helder::indent(ui);
            ui.label(RichText::new(e).color(t.flag));
        });
    }
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
    let frame = crate::helder::card_frame(t).inner_margin(Margin::same(24)).outer_margin(Margin { left: 6, right: 12, top: 0, bottom: 12 });
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
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
    if crate::ribbon_action_hint(ui, t, "✔", "Nieuwe taak", true, Some(&crate::key_hint("Shift+K"))) {
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

    #[test]
    fn a_repeat_goes_along_only_with_a_due_day_on_this_computer() {
        let mut e = Editor { title: "Weekrapport".into(), repeat: Some(Frequency::Weekly), interval: 2, ..Editor::default() };
        assert_eq!(e.params(), Err(REPEAT_NEEDS_DUE.to_string()), "no due day");
        e.due = "19-10-2026".into();
        assert_eq!(e.params().unwrap()["repeat"], json!({ "frequency": "weekly", "interval": 2 }));
        e.account_id = "acc".into();
        assert_eq!(e.params(), Err(REPEAT_NEEDS_DUE.to_string()), "a server would not know the repeat");
        e.account_id.clear();
        e.repeat = None;
        assert!(e.params().unwrap()["repeat"].is_null());
        let t =
            Task { title: "x".into(), due: Some(0), repeat: Some(Repeat { interval: 3, ..Repeat::every(Frequency::Monthly) }), ..Task::default() };
        let from = Editor::from_task(&t);
        assert_eq!((from.repeat, from.interval), (Some(Frequency::Monthly), 3));
    }
}
