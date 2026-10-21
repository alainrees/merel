//! Rules for incoming mail, kept in `rules.json` next to the accounts file and applied to the
//! messages a refresh finds new in an inbox, and to a whole folder when asked.
//!
//! A rule's effect is the same as the person doing it by hand: the header changes in the
//! session at once and a job for the server is queued, so it also works offline and is replayed
//! like any other change. The first rule that moves a message is the last one that sees it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use merel_mailbox::Flag;
use merel_model::{Rule, RuleError, RuleField};

use crate::{CommandError, Job, SAMPLE_ACCOUNT, Session};

/// What `rules.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesFile {
    pub rules: Vec<Rule>,
}

impl Session {
    pub fn set_rules(&mut self, rules: Vec<Rule>) {
        self.rules = rules;
    }

    pub fn rules_data(&self) -> RulesFile {
        RulesFile { rules: self.rules.clone() }
    }

    /// Whether a command changed the rules since the last call.
    pub fn take_rules_dirty(&mut self) -> bool {
        std::mem::take(&mut self.rules_dirty)
    }

    /// How many messages the rules changed since the last call, for a note in the status bar.
    pub fn take_rule_hits(&mut self) -> usize {
        std::mem::take(&mut self.rule_hits)
    }

    pub fn rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// The rules of an account, in the order they run.
    pub fn rules_of(&self, account_id: &str) -> Vec<&Rule> {
        self.rules.iter().filter(|r| r.account_id == account_id).collect()
    }

    /// Runs the enabled rules of the message's account over the message; true when one changed
    /// it. Nothing happens to a message outside an inbox unless `anywhere` is set (the "run
    /// now" button works on the open folder, whatever it is).
    pub(crate) fn apply_rules(&mut self, message_id: &str, anywhere: bool) -> bool {
        let Some(m) = self.message(message_id).cloned() else { return false };
        let Some(folder) = self.folder(&m.folder_id).cloned() else { return false };
        if !anywhere && folder.kind != merel_model::FolderKind::Inbox {
            return false;
        }
        let rules: Vec<Rule> = self.rules.iter().filter(|r| r.enabled && r.account_id == folder.account_id && r.matches(&m)).cloned().collect();
        let remote = (folder.account_id != SAMPLE_ACCOUNT).then(|| (folder.account_id.clone(), folder.path.clone(), m.remote_id.clone()));
        let mut changed = false;
        for rule in rules {
            if rule.mark_read && !self.flag_message(message_id, Flag::Seen, true, &remote) {
                continue;
            }
            changed |= rule.mark_read;
            if rule.flag {
                changed |= self.flag_message(message_id, Flag::Flagged, true, &remote);
            }
            if !rule.categories.is_empty()
                && let Some(current) = self.message(message_id).map(|m| m.categories.clone())
            {
                let mut all = current.clone();
                all.extend(rule.categories.iter().filter(|c| !current.iter().any(|x| x.eq_ignore_ascii_case(c))).cloned());
                if all.len() != current.len() && crate::categories::set_message_categories(self, message_id, all, Vec::new()).is_ok() {
                    changed = true;
                }
            }
            if let Some(target) =
                rule.move_to.as_deref().and_then(|id| self.folder(id)).filter(|t| t.account_id == folder.account_id && t.id != folder.id).cloned()
            {
                if let Some((account_id, path, id)) = &remote {
                    self.pending.push(Job::Move { account_id: account_id.clone(), path: path.clone(), id: id.clone(), dest: target.path.clone() });
                }
                if let Some(h) = self.messages.iter_mut().find(|h| h.id == message_id) {
                    h.folder_id = target.id;
                }
                if self.selected_message.as_deref() == Some(message_id) {
                    self.selected_message = None;
                }
                changed = true;
                break;
            }
        }
        if changed {
            self.rule_hits += 1;
        }
        changed
    }

    /// Sets a flag on a message as a rule asks, with the job for the server; false when the
    /// flag was already so.
    pub(crate) fn flag_message(&mut self, message_id: &str, flag: Flag, on: bool, remote: &Option<(String, String, String)>) -> bool {
        let Some(h) = self.messages.iter_mut().find(|h| h.id == message_id) else { return false };
        let slot = match flag {
            Flag::Seen => &mut h.flags.seen,
            Flag::Flagged => &mut h.flags.flagged,
        };
        if *slot == on {
            return false;
        }
        *slot = on;
        if let Some((account_id, path, id)) = remote {
            self.pending.push(Job::Flag { account_id: account_id.clone(), path: path.clone(), id: id.clone(), flag, on });
        }
        true
    }

    /// Runs the rules over the messages of a folder that are listed; how many changed.
    pub fn run_rules_in(&mut self, folder_id: &str) -> usize {
        let ids: Vec<String> = self.messages.iter().filter(|m| m.folder_id == folder_id).map(|m| m.id.clone()).collect();
        let before = self.rule_hits;
        for id in ids {
            self.apply_rules(&id, true);
        }
        let n = self.rule_hits - before;
        self.recount_unread();
        n
    }

    fn new_rule_id(&mut self) -> String {
        self.next_rule_id += 1;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("r-{now}-{}", self.next_rule_id)
    }
}

fn str_param<'a>(params: &'a Value, key: &'static str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

/// `rule.save { id?, name?, account_id?, field?, contains, move_to?, mark_read?, flag?, categories?, enabled? }`:
/// creates a rule (for the given account, else the current one) or replaces the one with `id`.
/// `field` is one of `from`, `to`, `subject`, `any`; `move_to` a folder id of the same account;
/// `categories` a list of names, spelled as the person's list has them when they are known.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let existing = match str_param(params, "id") {
        Some(id) => Some(s.rule(id).ok_or(CommandError::NotFound)?.clone()),
        None => None,
    };
    let account_id = match str_param(params, "account_id") {
        Some(a) => s.accounts.iter().find(|x| x.id == a).map(|x| x.id.clone()).ok_or(CommandError::Param("account_id"))?,
        None => existing
            .as_ref()
            .map(|r| r.account_id.clone())
            .or_else(|| s.current_account().map(|a| a.id.clone()))
            .ok_or(CommandError::Param("account_id"))?,
    };
    let field = match params.get("field") {
        None => existing.as_ref().map(|r| r.field).unwrap_or_default(),
        Some(v) => serde_json::from_value::<RuleField>(v.clone()).map_err(|_| CommandError::Param("field"))?,
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
    let bool_param = |key: &str, default: bool| params.get(key).and_then(Value::as_bool).unwrap_or(default);
    let rule = Rule {
        id: existing.as_ref().map(|r| r.id.clone()).unwrap_or_else(|| s.new_rule_id()),
        name: str_param(params, "name").unwrap_or("").to_string(),
        account_id,
        field,
        contains: str_param(params, "contains").or(existing.as_ref().map(|r| r.contains.as_str())).unwrap_or("").to_string(),
        move_to,
        mark_read: bool_param("mark_read", false),
        flag: bool_param("flag", false),
        categories,
        enabled: bool_param("enabled", existing.as_ref().is_none_or(|r| r.enabled)),
    }
    .validated()
    .map_err(|e| match e {
        RuleError::NoText => CommandError::Param("contains"),
        RuleError::NoAction => CommandError::Param("move_to"),
        RuleError::NoAccount => CommandError::Param("account_id"),
    })?;
    let id = rule.id.clone();
    match s.rules.iter_mut().find(|r| r.id == id) {
        Some(slot) => *slot = rule,
        None => s.rules.push(rule),
    }
    s.rules_dirty = true;
    Ok(())
}

/// `rule.delete { id }`.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").ok_or(CommandError::Param("id"))?;
    let before = s.rules.len();
    s.rules.retain(|r| r.id != id);
    if s.rules.len() == before {
        return Err(CommandError::NotFound);
    }
    s.rules_dirty = true;
    Ok(())
}

/// `rule.run { folder_id? }`: runs the rules over the listed messages of a folder, the selected
/// one when none is given. `take_rule_hits` tells how many messages changed.
pub fn run(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let folder_id = str_param(params, "folder_id").map(str::to_string).or_else(|| s.selected_folder.clone()).ok_or(CommandError::NoSelection)?;
    s.folder(&folder_id).ok_or(CommandError::NotFound)?;
    s.run_rules_in(&folder_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use merel_mailbox::{RemoteFolder, RemoteHeader};
    use merel_model::FolderKind;
    use serde_json::json;

    use super::*;
    use crate::{Event, HEADER_LIMIT};

    fn header(n: u32, from: &str, subject: &str) -> RemoteHeader {
        RemoteHeader {
            id: n.to_string(),
            seen: false,
            flagged: false,
            answered: false,
            size: 1,
            has_attachments: false,
            categories: Vec::new(),
            header: merel_mime::parse_header(format!("From: {from}\r\nSubject: {subject}\r\n\r\n").as_bytes()),
            date: i64::from(n),
        }
    }

    fn connected() -> Session {
        let mut s = Session::with_accounts(vec![crate::tests::account("a")]);
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![
                RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true },
                RemoteFolder { path: "Facturen".into(), name: "Facturen".into(), kind: FolderKind::Other, selectable: true },
            ],
            syncs_items: false,
        });
        s.take_jobs();
        s
    }

    #[test]
    fn rules_are_saved_run_on_new_inbox_mail_and_on_demand() {
        let mut s = connected();
        s.apply(Event::Headers {
            folder_id: "a/INBOX".into(),
            headers: vec![header(1, "sanne@example.org", "Factuur 1")],
            offset: 0,
            limit: HEADER_LIMIT,
        });
        s.run(
            "rule.save",
            &json!({ "name": "Facturen", "account_id": "a", "field": "subject", "contains": "factuur", "move_to": "a/Facturen", "mark_read": true }),
        )
        .unwrap();
        assert!(s.take_rules_dirty());
        assert_eq!(s.rules_data().rules.len(), 1);
        let id = s.rules[0].id.clone();
        assert_eq!(s.run("rule.save", &json!({ "account_id": "a", "contains": "x" })), Err(CommandError::Param("move_to")), "a rule needs an action");
        assert_eq!(
            s.run("rule.save", &json!({ "account_id": "a", "contains": "x", "flag": true, "move_to": "nope" })),
            Err(CommandError::Param("move_to"))
        );
        assert_eq!(
            s.run("rule.save", &json!({ "account_id": "a", "contains": "x", "flag": true, "field": "body" })),
            Err(CommandError::Param("field"))
        );
        assert_eq!(s.run("rule.save", &json!({ "account_id": "a", "mark_read": true })), Err(CommandError::Param("contains")));

        // Saving a rule touches nothing by itself; a refresh that brings new mail does.
        assert_eq!(s.message("a/INBOX#1").unwrap().folder_id, "a/INBOX");
        s.apply(Event::Headers {
            folder_id: "a/INBOX".into(),
            headers: vec![
                header(1, "sanne@example.org", "Factuur 1"),
                header(2, "jan@example.org", "Factuur 2"),
                header(3, "jan@example.org", "Hallo"),
            ],
            offset: 0,
            limit: HEADER_LIMIT,
        });
        assert_eq!(s.message("a/INBOX#2").unwrap().folder_id, "a/Facturen", "the new invoice moved");
        assert!(s.message("a/INBOX#2").unwrap().flags.seen);
        assert_eq!(s.message("a/INBOX#1").unwrap().folder_id, "a/INBOX", "mail that was already there is left alone");
        assert_eq!(s.message("a/INBOX#3").unwrap().folder_id, "a/INBOX");
        assert_eq!(s.take_rule_hits(), 1);
        let jobs = s.take_jobs();
        assert!(jobs.iter().any(|j| matches!(j, Job::Flag { id, flag: Flag::Seen, on: true, .. } if id == "2")));
        assert!(jobs.iter().any(|j| matches!(j, Job::Move { id, dest, .. } if id == "2" && dest == "Facturen")));
        assert_eq!(s.folder("a/INBOX").unwrap().unread, 2);

        // "Run now" goes over the whole folder, old mail included.
        s.run("rule.run", &json!({ "folder_id": "a/INBOX" })).unwrap();
        assert_eq!(s.take_rule_hits(), 1);
        assert_eq!(s.message("a/INBOX#1").unwrap().folder_id, "a/Facturen");
        s.run("rule.run", &json!({ "folder_id": "a/INBOX" })).unwrap();
        assert_eq!(s.take_rule_hits(), 0, "nothing left to do");

        // A disabled rule sleeps; a deleted one is gone.
        s.run("rule.save", &json!({ "id": id, "contains": "factuur", "move_to": "a/Facturen", "mark_read": true, "enabled": false })).unwrap();
        s.apply(Event::Headers {
            folder_id: "a/INBOX".into(),
            headers: vec![header(3, "jan@example.org", "Hallo"), header(4, "jan@example.org", "Factuur 4")],
            offset: 0,
            limit: HEADER_LIMIT,
        });
        assert_eq!(s.message("a/INBOX#4").unwrap().folder_id, "a/INBOX");
        assert_eq!(s.rules_of("a").len(), 1);
        s.run("rule.delete", &json!({ "id": id })).unwrap();
        assert!(s.rules.is_empty());
        assert_eq!(s.run("rule.delete", &json!({ "id": id })), Err(CommandError::NotFound));

        // Removing the account takes its rules along.
        s.run("rule.save", &json!({ "account_id": "a", "field": "from", "contains": "jan@", "flag": true })).unwrap();
        s.remove_account("a");
        assert!(s.rules.is_empty());
    }

    #[test]
    fn a_rule_gives_a_category_once_and_tells_the_server() {
        let mut s = connected();
        s.apply(Event::Headers { folder_id: "a/INBOX".into(), headers: vec![header(1, "jan@example.org", "Hallo")], offset: 0, limit: HEADER_LIMIT });
        s.run("rule.save", &json!({ "account_id": "a", "field": "from", "contains": "jan@", "categories": ["rode categorie", "Werk"] })).unwrap();
        assert_eq!(s.rules[0].categories, vec!["Rode categorie".to_string(), "Werk".to_string()], "a known name takes the list's spelling");
        assert_eq!(
            s.run("rule.save", &json!({ "account_id": "a", "contains": "x", "categories": "Werk" })),
            Err(CommandError::Param("categories")),
            "a list, not one name"
        );
        s.run("rule.run", &json!({ "folder_id": "a/INBOX" })).unwrap();
        assert_eq!(s.message("a/INBOX#1").unwrap().categories, vec!["Rode categorie".to_string(), "Werk".to_string()]);
        assert_eq!(s.take_rule_hits(), 1);
        let jobs = s.take_jobs();
        assert!(matches!(&jobs[..], [Job::Categories { id, categories, removed, .. }] if id == "1" && categories.len() == 2 && removed.is_empty()));
        s.run("rule.run", &json!({ "folder_id": "a/INBOX" })).unwrap();
        assert_eq!(s.take_rule_hits(), 0, "the message has them already");
        assert!(s.take_jobs().is_empty());
    }
}
