//! Tasks in the session and the commands that change them. Tasks live in `tasks.json` next to
//! the accounts file until they sync.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use mailcraft_model::{Priority, Task, TaskError};

use crate::{CommandError, Session};

/// What `tasks.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TasksFile {
    pub tasks: Vec<Task>,
}

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

    pub fn tasks_data(&self) -> TasksFile {
        TasksFile { tasks: self.tasks.clone() }
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

/// `task.save { id?, title, notes?, due?: seconds | null, priority?: "low" | "normal" | "high" }`:
/// creates the task, or replaces the one with `id` (keeping whether it is done).
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let existing = match str_param(params, "id") {
        Some(id) => Some(s.task(id).ok_or(CommandError::NotFound)?.clone()),
        None => None,
    };
    let priority = match params.get("priority") {
        None | Some(Value::Null) => existing.as_ref().map(|t| t.priority).unwrap_or_default(),
        Some(p) => p.as_str().and_then(Priority::parse).ok_or(CommandError::Param("priority"))?,
    };
    let due = match params.get("due") {
        None | Some(Value::Null) => None,
        Some(d) => Some(d.as_i64().ok_or(CommandError::Param("due"))?),
    };
    let id = existing.as_ref().map(|t| t.id.clone()).unwrap_or_else(|| s.new_task_id());
    let task = Task {
        id: id.clone(),
        title: str_param(params, "title").unwrap_or("").to_string(),
        notes: str_param(params, "notes").unwrap_or("").to_string(),
        due,
        priority,
        done: existing.as_ref().is_some_and(|t| t.done),
        completed: existing.as_ref().and_then(|t| t.completed),
    }
    .validated()
    .map_err(|e| match e {
        TaskError::NoTitle => CommandError::Param("title"),
    })?;
    s.tasks.retain(|t| t.id != id);
    s.tasks.push(task);
    s.selected_task = Some(id);
    s.tasks_dirty = true;
    Ok(())
}

/// `task.complete { id?, done? }`: marks the task done (or open again); without `done` it
/// flips.
pub fn complete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = selected_or_given(s, params)?;
    let t = s.tasks.iter_mut().find(|t| t.id == id).ok_or(CommandError::NotFound)?;
    t.done = params.get("done").and_then(Value::as_bool).unwrap_or(!t.done);
    t.completed = t.done.then(now);
    s.tasks_dirty = true;
    Ok(())
}

/// `task.delete { id? }`: the given task, else the selected one.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = selected_or_given(s, params)?;
    let before = s.tasks.len();
    s.tasks.retain(|t| t.id != id);
    if s.tasks.len() == before {
        return Err(CommandError::NotFound);
    }
    if s.selected_task.as_deref() == Some(id.as_str()) {
        s.selected_task = None;
    }
    s.tasks_dirty = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
