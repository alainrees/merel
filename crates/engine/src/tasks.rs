//! Tasks in the session and the commands that change them. Local tasks live in `tasks.json`
//! next to the accounts file; an account's live in its store and are mirrored here (see
//! [`crate::sync`]). A task with a due day reminds from eight in the morning of that day, as
//! Outlook does for tasks with a due date; a closed reminder is kept in `tasks.json` by the task
//! and the due day it was closed for, so a new due day reminds again.

use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use mailcraft_mailbox::Item;
use mailcraft_model::{Priority, Repeat, Task, TaskError};

use crate::{CommandError, Session};

/// What `tasks.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TasksFile {
    pub tasks: Vec<Task>,
    /// The task reminders that were closed, by task and due day.
    #[serde(default)]
    pub dismissed: Vec<TaskReminderKey>,
}

/// One task's reminder: the task and the due day it was set for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskReminderKey {
    pub id: String,
    pub due: i64,
}

/// The hour of the due day from which a task reminds, Outlook's default reminder time.
pub const TASK_REMINDER_HOUR: u32 = 8;

/// Which tasks a list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskFilter {
    Open,
    Done,
    All,
}

impl Session {
    pub fn set_tasks(&mut self, tasks: Vec<Task>) {
        self.tasks = tasks;
        self.selected_task = None;
    }

    /// The file's contents: the local tasks and the reminders closed so far.
    pub fn set_tasks_data(&mut self, data: TasksFile) {
        self.set_tasks(data.tasks);
        self.dismissed_task_reminders = data.dismissed;
    }

    /// What goes to the file: the local tasks, and the closed reminders of tasks still open on
    /// that due day.
    pub fn tasks_data(&self) -> TasksFile {
        let dismissed =
            self.dismissed_task_reminders.iter().filter(|k| self.task(&k.id).is_some_and(|t| !t.done && t.due == Some(k.due))).cloned().collect();
        TasksFile { tasks: self.tasks.iter().filter(|t| t.account_id.is_none()).cloned().collect(), dismissed }
    }

    /// The open tasks whose due day has come, Outlook's task reminders: from eight in the
    /// morning of the due day, and every day after until the task is done, unless the reminder
    /// was closed for that due day or put off until later than `now`. Earliest due first.
    pub fn due_task_reminders(&self, today: NaiveDate, now: i64) -> Vec<Task> {
        let after_eight = Local
            .timestamp_opt(now, 0)
            .single()
            .is_some_and(|t| t.time() >= NaiveTime::from_hms_opt(TASK_REMINDER_HOUR, 0, 0).unwrap_or(NaiveTime::MIN));
        let mut due: Vec<Task> = self
            .tasks
            .iter()
            .filter(|t| !t.done)
            .filter(|t| {
                let Some(due) = t.due else { return false };
                let Some(day) = Local.timestamp_opt(due, 0).single().map(|d| d.date_naive()) else { return false };
                (day < today || (day == today && after_eight))
                    && !self.dismissed_task_reminders.contains(&TaskReminderKey { id: t.id.clone(), due })
                    && !self.snoozed_task_reminders.get(&t.id).is_some_and(|until| *until > now)
            })
            .cloned()
            .collect();
        due.sort_by(|a, b| (a.due, &a.title).cmp(&(b.due, &b.title)));
        due
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn selected_task(&self) -> Option<&Task> {
        self.selected_task.as_deref().and_then(|id| self.task(id))
    }

    /// Tasks for the list: open ones first by due day (none last) and priority, done ones by
    /// completion, newest first.
    pub fn tasks_filtered(&self, filter: TaskFilter) -> Vec<&Task> {
        let mut list: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| match filter {
                TaskFilter::Open => !t.done,
                TaskFilter::Done => t.done,
                TaskFilter::All => true,
            })
            .collect();
        list.sort_by(|a, b| {
            a.done.cmp(&b.done).then_with(|| {
                if a.done {
                    b.completed.cmp(&a.completed)
                } else {
                    a.due.is_none().cmp(&b.due.is_none()).then(a.due.cmp(&b.due)).then(b.priority.cmp(&a.priority)).then(a.title.cmp(&b.title))
                }
            })
        });
        list
    }

    /// [`Self::tasks_filtered`] narrowed to the tasks whose title or notes hold every word of
    /// `query`; an empty query narrows nothing.
    pub fn tasks_matching(&self, filter: TaskFilter, query: &str) -> Vec<&Task> {
        let mut list = self.tasks_filtered(filter);
        if !query.trim().is_empty() {
            list.retain(|t| t.matches(query));
        }
        list
    }

    /// Whether a command changed the tasks since the last call.
    pub fn take_tasks_dirty(&mut self) -> bool {
        std::mem::take(&mut self.tasks_dirty)
    }

    fn new_task_id(&mut self) -> String {
        self.next_task_id += 1;
        format!("t-{}-{}", now(), self.next_task_id)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `task.reminder_dismiss { id }`: closes the task's reminder for its current due day.
pub fn reminder_dismiss(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").ok_or(CommandError::Param("id"))?.to_string();
    let due = s.task(&id).ok_or(CommandError::NotFound)?.due.ok_or(CommandError::Param("id"))?;
    s.snoozed_task_reminders.remove(&id);
    let key = TaskReminderKey { id, due };
    if !s.dismissed_task_reminders.contains(&key) {
        s.dismissed_task_reminders.push(key);
        s.tasks_dirty = true;
    }
    Ok(())
}

/// `task.reminder_snooze { id, minutes }`: puts the task's reminder off for `minutes` from now.
pub fn reminder_snooze(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").ok_or(CommandError::Param("id"))?.to_string();
    s.task(&id).ok_or(CommandError::NotFound)?;
    let minutes = params
        .get("minutes")
        .and_then(Value::as_u64)
        .filter(|m| (1..=u64::from(mailcraft_model::MAX_REMINDER)).contains(m))
        .ok_or(CommandError::Param("minutes"))?;
    s.snoozed_task_reminders.insert(id, now() + minutes as i64 * 60);
    Ok(())
}

fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn selected_or_given(s: &Session, params: &Value) -> Result<String, CommandError> {
    str_param(params, "id").map(str::to_string).or_else(|| s.selected_task.clone()).ok_or(CommandError::NoSelection)
}

/// `task.select { id }`.
pub fn select(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").ok_or(CommandError::Param("id"))?;
    s.task(id).ok_or(CommandError::NotFound)?;
    s.selected_task = Some(id.to_string());
    Ok(())
}

/// `task.save { id?, account_id?, title, notes?, due?: seconds | null, priority?: "low" | "normal" | "high",
/// repeat?: { frequency, interval } | null }`: creates the task (in the account's store when
/// `account_id` names one), or replaces the one with `id` (keeping whether it is done). A
/// repeat needs a due day and a task on this computer; left out, the existing repeat stays.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let existing = match str_param(params, "id") {
        Some(id) => Some(s.task(id).ok_or(CommandError::NotFound)?.clone()),
        None => None,
    };
    let account = match &existing {
        Some(t) => t.account_id.clone(),
        None => s.account_param(params)?,
    };
    let priority = match params.get("priority") {
        None | Some(Value::Null) => existing.as_ref().map(|t| t.priority).unwrap_or_default(),
        Some(p) => p.as_str().and_then(Priority::parse).ok_or(CommandError::Param("priority"))?,
    };
    let due = match params.get("due") {
        None | Some(Value::Null) => None,
        Some(d) => Some(d.as_i64().ok_or(CommandError::Param("due"))?),
    };
    let repeat = match params.get("repeat") {
        None => existing.as_ref().and_then(|t| t.repeat),
        Some(Value::Null) => None,
        Some(r) => Some(serde_json::from_value::<Repeat>(r.clone()).map_err(|_| CommandError::Param("repeat"))?),
    };
    let id = match (&existing, &account) {
        (Some(t), _) => t.id.clone(),
        (None, Some(account)) => s.pending_id(account),
        (None, None) => s.new_task_id(),
    };
    let task = Task {
        id: id.clone(),
        title: str_param(params, "title").unwrap_or("").to_string(),
        notes: str_param(params, "notes").unwrap_or("").to_string(),
        due,
        priority,
        done: existing.as_ref().is_some_and(|t| t.done),
        completed: existing.as_ref().and_then(|t| t.completed),
        account_id: account.clone(),
        repeat,
        // Formatting that does not parse is dropped: the notes stay, as text.
        notes_rich: params.get("notes_rich").and_then(|v| serde_json::from_value(v.clone()).ok()),
    }
    .validated()
    .map_err(|e| match e {
        TaskError::NoTitle => CommandError::Param("title"),
        TaskError::RepeatNeedsDue => CommandError::Param("repeat"),
    })?;
    match &account {
        Some(account) => s.save_remote(account, existing.is_none(), Item::Task(task.clone()))?,
        None => s.tasks_dirty = true,
    }
    s.tasks.retain(|t| t.id != id);
    s.tasks.push(task);
    s.selected_task = Some(id);
    Ok(())
}

/// `task.complete { id?, done? }`: marks the task done (or open again); without `done` it
/// flips. Finishing a repeating task adds the next one, due a repeat later, as Outlook does;
/// the finished one stays as it is.
pub fn complete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = selected_or_given(s, params)?;
    let mut task = s.task(&id).ok_or(CommandError::NotFound)?.clone();
    let was_done = task.done;
    task.done = params.get("done").and_then(Value::as_bool).unwrap_or(!task.done);
    task.completed = task.done.then(now);
    match task.account_id.clone() {
        Some(account) => s.save_remote(&account, false, Item::Task(task.clone()))?,
        None => s.tasks_dirty = true,
    }
    let next = (task.done && !was_done).then(|| next_task(s, &task)).flatten();
    if let Some(t) = s.tasks.iter_mut().find(|t| t.id == id) {
        *t = task;
    }
    if let Some(next) = next {
        s.tasks_dirty = true;
        s.tasks.push(next);
    }
    Ok(())
}

/// The task that follows a finished repeating one: the same task, open, due a repeat later.
fn next_task(s: &mut Session, done: &Task) -> Option<Task> {
    let (Some(repeat), Some(due)) = (done.repeat, done.due) else { return None };
    let due = repeat.next_due(due, &chrono::Local)?;
    Some(Task { id: s.new_task_id(), due: Some(due), done: false, completed: None, ..done.clone() })
}

/// `task.delete { id? }`: the given task, else the selected one.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = selected_or_given(s, params)?;
    let account = s.task(&id).ok_or(CommandError::NotFound)?.account_id.clone();
    s.tasks.retain(|t| t.id != id);
    if s.selected_task.as_deref() == Some(id.as_str()) {
        s.selected_task = None;
    }
    match account {
        Some(account) => s.delete_remote(&account, &id),
        None => s.tasks_dirty = true,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn finishing_a_repeating_task_makes_the_next_one_a_fortnight_later() {
        use chrono::{Days, Local, NaiveDate, NaiveTime, TimeZone};
        let midnight = |d: NaiveDate| Local.from_local_datetime(&d.and_time(NaiveTime::MIN)).earliest().unwrap().timestamp();
        let day = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let mut s = Session::default();
        let weekly = json!({ "title": "Weekrapport", "due": midnight(day), "repeat": { "frequency": "weekly", "interval": 2 } });
        s.run("task.save", &weekly).unwrap();
        assert_eq!(
            s.run("task.save", &json!({ "title": "x", "repeat": { "frequency": "daily" } })),
            Err(CommandError::Param("repeat")),
            "no due day"
        );
        let id = s.tasks[0].id.clone();
        s.run("task.complete", &json!({ "id": id })).unwrap();
        assert_eq!(s.tasks.len(), 2, "the next one is added");
        let next = s.tasks.iter().find(|t| t.id != id).unwrap();
        assert_eq!((next.title.as_str(), next.done, next.due), ("Weekrapport", false, Some(midnight(day.checked_add_days(Days::new(14)).unwrap()))));
        assert_eq!(next.repeat, s.tasks[0].repeat, "it repeats on");
        assert!(s.tasks[0].done);
        s.run("task.complete", &json!({ "id": id })).unwrap();
        s.run("task.complete", &json!({ "id": id })).unwrap();
        assert_eq!(s.tasks.len(), 3, "reopening and finishing again adds another");
    }

    #[test]
    fn the_search_box_narrows_the_list_to_tasks_holding_every_word() {
        let mut s = Session::default();
        s.run("task.save", &json!({ "title": "Offerte drukwerk", "notes": "voor Ravi" })).unwrap();
        s.run("task.save", &json!({ "title": "Portfolio bijwerken" })).unwrap();
        s.run("task.save", &json!({ "title": "Offerte sturen" })).unwrap();
        s.run("task.complete", &Value::Null).unwrap();
        let titles = |list: Vec<&Task>| list.into_iter().map(|t| t.title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(s.tasks_matching(TaskFilter::Open, "  ")).len(), 2, "blank narrows nothing");
        assert_eq!(titles(s.tasks_matching(TaskFilter::Open, "OFFERTE")), ["Offerte drukwerk"], "case does not matter; the filter still holds");
        assert_eq!(titles(s.tasks_matching(TaskFilter::All, "offerte")), ["Offerte drukwerk", "Offerte sturen"]);
        assert_eq!(titles(s.tasks_matching(TaskFilter::Open, "ravi offerte")), ["Offerte drukwerk"], "every word, title or notes");
        assert!(s.tasks_matching(TaskFilter::Open, "belasting").is_empty());
    }

    #[test]
    fn a_task_reminds_from_its_due_day_until_done_closed_or_put_off() {
        use chrono::{Days, Local, NaiveTime, TimeZone};
        let mut s = Session::default();
        let today = Local::now().date_naive();
        let midnight = |d: chrono::NaiveDate| Local.from_local_datetime(&d.and_time(NaiveTime::MIN)).earliest().map(|t| t.timestamp()).unwrap();
        let yesterday = today.checked_sub_days(Days::new(1)).unwrap();
        let tomorrow = today.checked_add_days(Days::new(1)).unwrap();
        s.run("task.save", &json!({ "title": "Aangifte", "due": midnight(yesterday) })).unwrap();
        let late = s.selected_task().unwrap().id.clone();
        s.run("task.save", &json!({ "title": "Later", "due": midnight(tomorrow) })).unwrap();
        s.run("task.save", &json!({ "title": "Ooit" })).unwrap();
        s.run("task.save", &json!({ "title": "Vandaag", "due": midnight(today) })).unwrap();
        let noon = midnight(today) + 12 * 3600;
        let early = midnight(today) + 7 * 3600;
        let titles = |s: &Session, now: i64| s.due_task_reminders(today, now).iter().map(|t| t.title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(&s, noon), ["Aangifte", "Vandaag"], "overdue and due today at noon; not tomorrow's, not one without a day");
        assert_eq!(titles(&s, early), ["Aangifte"], "before eight only the overdue one");

        s.run("task.reminder_snooze", &json!({ "id": late, "minutes": 60 })).unwrap();
        assert!(!titles(&s, now()).contains(&"Aangifte".to_string()), "put off");
        assert!(titles(&s, now() + 3601).contains(&"Aangifte".to_string()), "and back after an hour");
        assert_eq!(s.run("task.reminder_snooze", &json!({ "id": late, "minutes": 0 })), Err(CommandError::Param("minutes")));
        assert_eq!(s.run("task.reminder_dismiss", &json!({ "id": "nope" })), Err(CommandError::NotFound));

        s.take_tasks_dirty();
        s.run("task.reminder_dismiss", &json!({ "id": late })).unwrap();
        assert!(s.take_tasks_dirty(), "closing is kept in the file");
        assert_eq!(titles(&s, noon), ["Vandaag"], "closed for this due day");
        let saved = s.tasks_data();
        assert_eq!(saved.dismissed, vec![TaskReminderKey { id: late.clone(), due: midnight(yesterday) }]);
        let mut again = Session::default();
        again.set_tasks_data(saved);
        assert_eq!(titles(&again, noon), ["Vandaag"], "the file remembers what was closed");

        s.run("task.save", &json!({ "id": late, "title": "Aangifte", "due": midnight(today) })).unwrap();
        assert_eq!(titles(&s, noon), ["Aangifte", "Vandaag"], "a new due day reminds again");
        s.run("task.complete", &json!({ "id": late, "done": true })).unwrap();
        assert_eq!(titles(&s, noon), ["Vandaag"], "done is done");
        s.run("task.reminder_dismiss", &json!({ "id": late })).unwrap();
        assert!(s.tasks_data().dismissed.is_empty(), "a closed reminder of a finished task is not kept");
    }

    #[test]
    fn tasks_are_saved_completed_sorted_and_deleted() {
        let mut s = Session::default();
        s.run("task.save", &json!({ "title": "Laat", "due": 200 })).unwrap();
        s.run("task.save", &json!({ "title": "Vroeg", "due": 100, "priority": "low" })).unwrap();
        s.run("task.save", &json!({ "title": "Ooit" })).unwrap();
        let ooit = s.selected_task().unwrap().id.clone();
        s.run("task.save", &json!({ "title": "Vroeg en belangrijk", "due": 100, "priority": "high" })).unwrap();
        assert!(s.take_tasks_dirty());
        let titles: Vec<&str> = s.tasks_filtered(TaskFilter::Open).iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Vroeg en belangrijk", "Vroeg", "Laat", "Ooit"]);

        s.run("task.select", &json!({ "id": ooit })).unwrap();
        s.run("task.complete", &Value::Null).unwrap();
        assert!(s.task(&ooit).unwrap().done && s.task(&ooit).unwrap().completed.is_some());
        assert_eq!(s.tasks_filtered(TaskFilter::Done).len(), 1);
        assert_eq!(s.tasks_filtered(TaskFilter::All).last().map(|t| t.title.as_str()), Some("Ooit"));
        s.run("task.complete", &json!({ "id": ooit, "done": false })).unwrap();
        assert!(!s.task(&ooit).unwrap().done);

        s.run("task.save", &json!({ "id": ooit, "title": "Ooit eens", "notes": "n" })).unwrap();
        assert_eq!(s.task(&ooit).unwrap().priority, Priority::Normal);
        assert_eq!(s.run("task.save", &json!({ "title": " " })), Err(CommandError::Param("title")));
        assert_eq!(s.run("task.save", &json!({ "title": "x", "priority": "urgent" })), Err(CommandError::Param("priority")));
        assert_eq!(s.run("task.select", &json!({ "id": "nope" })), Err(CommandError::NotFound));

        s.run("task.delete", &json!({ "id": ooit })).unwrap();
        assert_eq!(s.tasks.len(), 3);
        assert!(s.selected_task.is_none());
        assert_eq!(s.run("task.delete", &Value::Null), Err(CommandError::NoSelection));
    }
}
