//! Quick steps: the person's own buttons that do several things to the selected message at
//! once, kept in `quick_steps.json` next to the accounts file. Running one has the same effect
//! as doing each thing by hand: the session changes at once and the jobs for the server are
//! queued, so a step works offline like any other change.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use mailcraft_mailbox::Flag;
use mailcraft_model::{QuickStep, QuickStepError};

use crate::commands::{self, CommandError, str_param};
use crate::{SAMPLE_ACCOUNT, Session};

/// What `quick_steps.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickStepsFile {
    pub quick_steps: Vec<QuickStep>,
}

impl Session {
    pub fn set_quick_steps(&mut self, steps: Vec<QuickStep>) {
        self.quick_steps = steps;
    }

    pub fn quick_steps_data(&self) -> QuickStepsFile {
        QuickStepsFile { quick_steps: self.quick_steps.clone() }
    }

    /// Whether a command changed the quick steps since the last call.
    pub fn take_quick_steps_dirty(&mut self) -> bool {
        std::mem::take(&mut self.quick_steps_dirty)
    }

    pub fn quick_step(&self, id: &str) -> Option<&QuickStep> {
        self.quick_steps.iter().find(|q| q.id == id)
    }

    /// The quick steps of an account, in the order they are shown.
    pub fn quick_steps_of(&self, account_id: &str) -> Vec<&QuickStep> {
        self.quick_steps.iter().filter(|q| q.account_id == account_id).collect()
    }

    fn new_quick_step_id(&mut self) -> String {
        self.next_quick_step_id += 1;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("q-{now}-{}", self.next_quick_step_id)
    }
}

/// `quickstep.save { id?, name, account_id?, move_to?, mark_read?, flag?, categories? }`: creates
/// a quick step (for the given account, else the current one) or replaces the one with `id`.
/// `move_to` is a folder id of the same account; `categories` a list of names, spelled as the
/// person's list has them when they are known.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let existing = match str_param(params, "id") {
        Ok(id) => Some(s.quick_step(id).ok_or(CommandError::NotFound)?.clone()),
        Err(_) => None,
    };
    let account_id = match params.get("account_id").and_then(Value::as_str) {
        Some(a) => s.accounts.iter().find(|x| x.id == a).map(|x| x.id.clone()).ok_or(CommandError::Param("account_id"))?,
        None => existing
            .as_ref()
            .map(|q| q.account_id.clone())
            .or_else(|| s.current_account().map(|a| a.id.clone()))
            .ok_or(CommandError::Param("account_id"))?,
    };
    let move_to = match params.get("move_to") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let id = v.as_str().ok_or(CommandError::Param("move_to"))?;
            let folder = s.folder(id).ok_or(CommandError::Param("move_to"))?;
            if folder.account_id != account_id {
                return Err(CommandError::Param("move_to"));
            }
            Some(folder.id.clone())
        }
    };
    let categories = match params.get("categories") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str().map(|c| {
                    s.categories.iter().find(|k| k.name.eq_ignore_ascii_case(c.trim())).map(|k| k.name.clone()).unwrap_or_else(|| c.to_string())
                })
            })
            .collect::<Option<Vec<String>>>()
            .ok_or(CommandError::Param("categories"))?,
        Some(_) => return Err(CommandError::Param("categories")),
    };
    let bool_param = |key: &str| params.get(key).and_then(Value::as_bool).unwrap_or(false);
    let step = QuickStep {
        id: existing.as_ref().map(|q| q.id.clone()).unwrap_or_else(|| s.new_quick_step_id()),
        name: params.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        account_id,
        move_to,
        mark_read: bool_param("mark_read"),
        flag: bool_param("flag"),
        categories,
    }
    .validated()
    .map_err(|e| match e {
        QuickStepError::NoName => CommandError::Param("name"),
        QuickStepError::NoAction => CommandError::Param("move_to"),
        QuickStepError::NoAccount => CommandError::Param("account_id"),
    })?;
    let id = step.id.clone();
    match s.quick_steps.iter_mut().find(|q| q.id == id) {
        Some(slot) => *slot = step,
        None => s.quick_steps.push(step),
    }
    s.quick_steps_dirty = true;
    Ok(())
}

/// `quickstep.delete { id }`.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?;
    let before = s.quick_steps.len();
    s.quick_steps.retain(|q| q.id != id);
    if s.quick_steps.len() == before {
        return Err(CommandError::NotFound);
    }
    s.quick_steps_dirty = true;
    Ok(())
}

/// `quickstep.run { id, message_id? }`: does what the step says to the message (the selected one
/// when none is given): marks and categories first, the move last. A step of another account
/// does nothing (`Param("id")`); a folder that no longer exists leaves the message where it is
/// after the rest is done (`Param("move_to")`).
pub fn run(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let step = s.quick_step(str_param(params, "id")?).ok_or(CommandError::NotFound)?.clone();
    let message_id = params
        .get("message_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| s.selected_message.clone())
        .ok_or(CommandError::NoSelection)?;
    let m = s.message(&message_id).cloned().ok_or(CommandError::NotFound)?;
    let folder = s.folder(&m.folder_id).cloned().ok_or(CommandError::NotFound)?;
    if folder.account_id != step.account_id {
        return Err(CommandError::Param("id"));
    }
    let remote = (folder.account_id != SAMPLE_ACCOUNT).then(|| (folder.account_id.clone(), folder.path.clone(), m.remote_id.clone()));
    if step.mark_read {
        s.flag_message(&message_id, Flag::Seen, true, &remote);
    }
    if step.flag {
        s.flag_message(&message_id, Flag::Flagged, true, &remote);
    }
    if !step.categories.is_empty() {
        let mut all = m.categories.clone();
        all.extend(step.categories.iter().filter(|c| !m.categories.iter().any(|x| x.eq_ignore_ascii_case(c))).cloned());
        if all.len() != m.categories.len() {
            crate::categories::set_message_categories(s, &message_id, all, Vec::new())?;
        }
    }
    s.recount_unread();
    match step.move_to.as_deref() {
        Some(target) if target != folder.id => {
            if s.folder(target).is_none() {
                return Err(CommandError::Param("move_to"));
            }
            commands::move_message(s, &message_id, target)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Job;

    #[test]
    fn a_quick_step_is_saved_run_on_the_selected_message_and_removed() {
        let mut s = Session::sample();
        assert_eq!(s.run("quickstep.save", &json!({ "name": "Klaar" })), Err(CommandError::Param("move_to")), "a step needs an action");
        assert_eq!(s.run("quickstep.save", &json!({ "mark_read": true })), Err(CommandError::Param("name")));
        assert_eq!(s.run("quickstep.save", &json!({ "name": "X", "move_to": "nope" })), Err(CommandError::Param("move_to")));
        s.run("quickstep.save", &json!({ "name": "Klaar", "move_to": "archive", "mark_read": true, "categories": ["rode categorie"] }))
            .expect("save");
        assert!(s.take_quick_steps_dirty());
        let id = s.quick_steps[0].id.clone();
        assert_eq!((s.quick_steps[0].account_id.as_str(), s.quick_steps[0].categories.as_slice()), ("sample", &["Rode categorie".to_string()][..]));
        assert_eq!(s.quick_steps_of("sample").len(), 1);

        assert_eq!(s.run("quickstep.run", &json!({ "id": id })), Err(CommandError::NoSelection));
        s.run("message.select", &json!({ "id": "m0" })).expect("select");
        s.run("message.mark_unread", &json!({})).expect("unread again");
        s.run("quickstep.run", &json!({ "id": id })).expect("run");
        let m = s.message("m0").expect("m0");
        assert_eq!((m.folder_id.as_str(), m.flags.seen, m.categories.as_slice()), ("archive", true, &["Rode categorie".to_string()][..]));
        assert!(s.take_jobs().is_empty(), "the sample account has no server");

        // Running it again where the message already is changes nothing and is no error.
        s.run("message.select", &json!({ "id": "m0" })).expect("select");
        s.run("quickstep.run", &json!({ "id": id })).expect("run again");
        assert_eq!(s.message("m0").map(|m| m.categories.len()), Some(1));

        // A step whose folder is gone does the rest and says so.
        s.run("quickstep.save", &json!({ "id": id, "name": "Klaar", "move_to": "junk", "flag": true })).expect("change");
        s.folders.retain(|f| f.id != "junk");
        s.run("message.select", &json!({ "id": "m1" })).expect("select");
        assert_eq!(s.run("quickstep.run", &json!({ "id": id })), Err(CommandError::Param("move_to")));
        assert!(s.message("m1").is_some_and(|m| m.flags.flagged && m.folder_id == "inbox"));

        s.run("quickstep.delete", &json!({ "id": id })).expect("delete");
        assert!(s.quick_steps.is_empty());
        assert_eq!(s.run("quickstep.delete", &json!({ "id": id })), Err(CommandError::NotFound));
    }

    #[test]
    fn a_real_account_gets_the_jobs_and_a_step_stays_with_its_account() {
        use mailcraft_mailbox::RemoteFolder;
        use mailcraft_model::FolderKind;

        let mut s = Session::with_accounts(vec![crate::tests::account("acc")]);
        let folders = vec![
            RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true },
            RemoteFolder { path: "Klaar".into(), name: "Klaar".into(), kind: FolderKind::Other, selectable: true },
        ];
        s.apply(crate::Event::Connected { account_id: "acc".into(), folders, syncs_items: false });
        let mut h = crate::tests::header(7);
        h.seen = false;
        s.apply(crate::Event::Headers { folder_id: "acc/INBOX".into(), headers: vec![h], offset: 0, limit: crate::HEADER_LIMIT });
        s.take_jobs();
        s.run("quickstep.save", &json!({ "name": "Afgehandeld", "account_id": "acc", "move_to": "acc/Klaar", "mark_read": true })).expect("save");
        let id = s.quick_steps[0].id.clone();
        s.run("quickstep.run", &json!({ "id": id, "message_id": "acc/INBOX#7" })).expect("run");
        let jobs = s.take_jobs();
        assert!(matches!(&jobs[..], [Job::Flag { flag: Flag::Seen, on: true, .. }, Job::Move { dest, .. }] if dest == "Klaar"), "{jobs:?}");
        assert_eq!(s.message("acc/INBOX#7").map(|m| m.folder_id.clone()), Some("acc/Klaar".to_string()));
        s.remove_account("acc");
        assert!(s.quick_steps.is_empty(), "removing the account takes its steps along");
    }
}
