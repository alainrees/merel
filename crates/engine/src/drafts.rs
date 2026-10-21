//! Messages put aside while being written. They live in the session and, per account, in the
//! encrypted offline cache, so a half-written mail survives a restart without ever reaching a
//! plain file. Nothing goes to the server: a draft is this computer's until it is sent.
//!
//! A draft with `send_at` is a scheduled message (Outlook's "Bezorging uitstellen"): the engine
//! sends it when the moment comes, while the app runs. It stays a draft until the server took
//! it, and one that cannot go becomes an ordinary draft again with a word for the person.

use serde_json::Value;

use merel_model::StoredDraft;

use crate::{CommandError, SAMPLE_ACCOUNT, SendState, Session};

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Session {
    /// The drafts of `account_id`, the one touched last first.
    pub fn drafts_of(&self, account_id: &str) -> Vec<&StoredDraft> {
        let mut list: Vec<&StoredDraft> = self.drafts.iter().filter(|d| d.account_id == account_id).collect();
        list.sort_by_key(|d| std::cmp::Reverse(d.updated));
        list
    }

    pub fn draft(&self, id: &str) -> Option<&StoredDraft> {
        self.drafts.iter().find(|d| d.id == id)
    }

    /// Puts `draft` aside: a new one gets an id and the current account when it names none; one
    /// with a known id replaces that one. The id lands in `last_draft` for the window.
    pub fn save_draft(&mut self, mut draft: StoredDraft) -> Result<String, CommandError> {
        if draft.account_id.is_empty() {
            draft.account_id = self.current_account().map(|a| a.id.clone()).ok_or(CommandError::Param("account_id"))?;
        }
        if !self.accounts.iter().any(|a| a.id == draft.account_id) {
            return Err(CommandError::Param("account_id"));
        }
        if draft.id.is_empty() || self.draft(&draft.id).is_none() {
            self.next_stored_draft += 1;
            draft.id = format!("d-{}-{}", now(), self.next_stored_draft);
        }
        draft.updated = now();
        let draft = draft.cleaned();
        let id = draft.id.clone();
        self.drafts.retain(|d| d.id != id);
        self.drafts.push(draft);
        self.last_draft = Some(id.clone());
        Ok(id)
    }

    /// Whether anything of a sample account's would be lost: the sample account has no cache.
    pub fn drafts_are_kept(&self, account_id: &str) -> bool {
        account_id != SAMPLE_ACCOUNT
    }

    /// The scheduled messages whose moment has come and that are not on their way yet.
    pub fn due_scheduled(&self, now: i64) -> Vec<String> {
        self.drafts
            .iter()
            .filter(|d| d.send_at.is_some_and(|t| t <= now) && !self.scheduled_sends.values().any(|id| *id == d.id))
            .map(|d| d.id.clone())
            .collect()
    }

    /// Sends a scheduled message now. The draft stays until the server took the message; one
    /// that cannot go at all becomes an ordinary draft again.
    pub fn send_scheduled(&mut self, id: &str) {
        let Some(d) = self.draft(id).cloned() else { return };
        match d.to_draft() {
            Ok(draft) => {
                let draft_id = self.send(&d.account_id, draft);
                self.scheduled_sends.insert(draft_id, id.to_string());
                // Without an account to send from, the outbox says so at once.
                if let Some(SendState::Failed(e)) = self.outbox.get(&draft_id).cloned() {
                    self.settle_scheduled(draft_id, Err(e));
                }
            }
            Err(e) => self.keep_as_draft(id, e),
        }
    }

    /// What became of a scheduled message's send: gone when it went, a plain draft when it did
    /// not. False for a message that was not scheduled.
    pub(crate) fn settle_scheduled(&mut self, draft_id: u64, result: Result<(), String>) -> bool {
        let Some(id) = self.scheduled_sends.remove(&draft_id) else { return false };
        match result {
            Ok(()) => {
                self.drafts.retain(|d| d.id != id);
                if self.last_draft.as_deref() == Some(id.as_str()) {
                    self.last_draft = None;
                }
            }
            Err(e) => self.keep_as_draft(&id, e),
        }
        true
    }

    fn keep_as_draft(&mut self, id: &str, why: String) {
        let Some(d) = self.drafts.iter_mut().find(|d| d.id == id) else { return };
        d.send_at = None;
        let subject = if d.subject.is_empty() { merel_i18n::tr("no-subject-capital") } else { d.subject.clone() };
        self.scheduled_failures.push(merel_i18n::tr_args("scheduled-failed", &[("subject", subject.into()), ("why", why.to_string().into())]));
    }

    /// What went wrong with scheduled messages since the last call, for the status bar.
    pub fn take_scheduled_failures(&mut self) -> Vec<String> {
        std::mem::take(&mut self.scheduled_failures)
    }
}

/// `draft.schedule { id, send_at? }`: sends the draft at `send_at` (seconds since the Unix
/// epoch); without `send_at` the draft is an ordinary one again.
pub fn schedule(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = params.get("id").and_then(Value::as_str).ok_or(CommandError::Param("id"))?;
    let send_at = match params.get("send_at") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or(CommandError::Param("send_at"))?),
    };
    let d = s.drafts.iter_mut().find(|d| d.id == id).ok_or(CommandError::NotFound)?;
    d.send_at = send_at;
    Ok(())
}

/// `draft.save { id?, account_id?, to, cc, subject, body, in_reply_to?, attachments? }`.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let draft: StoredDraft = serde_json::from_value(params.clone()).map_err(|_| CommandError::Param("draft"))?;
    s.save_draft(draft).map(drop)
}

/// `draft.delete { id }`.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = params.get("id").and_then(Value::as_str).ok_or(CommandError::Param("id"))?;
    let before = s.drafts.len();
    s.drafts.retain(|d| d.id != id);
    if s.drafts.len() == before {
        return Err(CommandError::NotFound);
    }
    if s.last_draft.as_deref() == Some(id) {
        s.last_draft = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn drafts_are_saved_replaced_listed_deleted_and_cached_per_account() {
        let mut s = Session::sample();
        let sample = s.accounts[0].id.clone();
        s.run("draft.save", &json!({ "to": "sanne@", "cc": "", "subject": " Plan\u{7} ", "body": "Hoi\n\u{7}", "in_reply_to": null })).unwrap();
        let id = s.last_draft.clone().expect("an id for the window");
        let d = s.draft(&id).unwrap().clone();
        assert_eq!((d.account_id.as_str(), d.to.as_str(), d.subject.as_str(), d.body.as_str()), (sample.as_str(), "sanne@", "Plan", "Hoi\n"));
        assert!(d.updated > 0 && !d.is_blank());

        s.run("draft.save", &json!({ "id": id, "to": "sanne@voorbeeld.nl", "cc": "", "subject": "Plan", "body": "Hoi" })).unwrap();
        assert_eq!(s.drafts.len(), 1, "a known id replaces");
        assert_eq!(s.drafts_of(&sample)[0].to, "sanne@voorbeeld.nl");
        s.run("draft.save", &json!({ "id": "stale", "to": "", "cc": "", "subject": "Tweede", "body": "" })).unwrap();
        assert_eq!(s.drafts.len(), 2, "an unknown id is a new draft");
        assert_ne!(s.last_draft.as_deref(), Some("stale"));
        assert_eq!(s.drafts_of(&sample).len(), 2);
        assert_eq!(
            s.run("draft.save", &json!({ "account_id": "nope", "to": "", "cc": "", "subject": "x", "body": "" })),
            Err(CommandError::Param("account_id"))
        );

        assert_eq!(s.run("draft.delete", &json!({ "id": "nope" })), Err(CommandError::NotFound));
        s.run("draft.delete", &json!({ "id": id })).unwrap();
        assert_eq!(s.drafts.len(), 1);

        // Real accounts keep their drafts in the cache and get them back; removal drops them.
        let mut s = Session::with_accounts(vec![crate::tests::account("a"), crate::tests::account("b")]);
        s.run("draft.save", &json!({ "account_id": "a", "to": "", "cc": "", "subject": "Van a", "body": "" })).unwrap();
        s.run("draft.save", &json!({ "account_id": "b", "to": "", "cc": "", "subject": "Van b", "body": "" })).unwrap();
        let cached = s.snapshot("a");
        assert_eq!(cached.drafts.iter().map(|d| d.subject.as_str()).collect::<Vec<_>>(), ["Van a"]);
        let mut fresh = Session::with_accounts(vec![crate::tests::account("a")]);
        fresh.restore(&cached);
        assert_eq!(fresh.drafts_of("a").len(), 1);
        s.remove_account("a");
        assert_eq!(s.drafts.iter().map(|d| d.subject.as_str()).collect::<Vec<_>>(), ["Van b"]);
    }

    #[test]
    fn a_scheduled_message_goes_out_at_its_moment_and_comes_back_as_a_draft_when_it_cannot() {
        use crate::{Event, Job};
        let mut s = Session::with_accounts(vec![crate::tests::account("a")]);
        s.run("draft.save", &json!({ "account_id": "a", "to": "sanne@voorbeeld.nl", "cc": "", "subject": "Later", "body": "Tot dan" })).unwrap();
        let id = s.last_draft.clone().unwrap();
        assert_eq!(s.run("draft.schedule", &json!({ "id": "nope", "send_at": 1000 })), Err(CommandError::NotFound));
        assert_eq!(s.run("draft.schedule", &json!({ "id": id, "send_at": "soon" })), Err(CommandError::Param("send_at")));
        s.run("draft.schedule", &json!({ "id": id, "send_at": 1000 })).unwrap();
        assert_eq!(s.draft(&id).unwrap().send_at, Some(1000));
        assert!(s.due_scheduled(999).is_empty());
        assert_eq!(s.due_scheduled(1000), std::slice::from_ref(&id));

        s.take_jobs();
        s.send_scheduled(&id);
        let jobs = s.take_jobs();
        let Some(Job::Send { draft_id, draft, .. }) = jobs.iter().find(|j| matches!(j, Job::Send { .. })) else { panic!("{jobs:?}") };
        assert_eq!((draft.subject.as_str(), draft.to[0].email()), ("Later", "sanne@voorbeeld.nl"));
        assert!(s.draft(&id).is_some(), "the draft stays until the server took the message");
        assert!(s.due_scheduled(5000).is_empty(), "a message on its way is not sent twice");
        s.apply(Event::Sent { account_id: "a".into(), draft_id: *draft_id, result: Err("Geen verbinding.".into()) });
        let d = s.draft(&id).expect("kept as a draft");
        assert_eq!(d.send_at, None);
        let failures = s.take_scheduled_failures();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("\"Later\"") && failures[0].ends_with("Geen verbinding."), "{failures:?}");

        s.run("draft.schedule", &json!({ "id": id, "send_at": 2000 })).unwrap();
        s.send_scheduled(&id);
        let Some(Job::Send { draft_id, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else { panic!("no send") };
        s.apply(Event::Sent { account_id: "a".into(), draft_id, result: Ok(()) });
        assert!(s.draft(&id).is_none(), "sent, so gone");
        assert!(s.scheduled_sends.is_empty() && s.take_scheduled_failures().is_empty());

        // One without a recipient cannot go, and says so.
        s.run("draft.save", &json!({ "account_id": "a", "to": "", "cc": "", "subject": "", "body": "x" })).unwrap();
        let id = s.last_draft.clone().unwrap();
        s.run("draft.schedule", &json!({ "id": id, "send_at": 1 })).unwrap();
        s.send_scheduled(&id);
        assert!(s.take_jobs().is_empty());
        assert_eq!(s.draft(&id).unwrap().send_at, None);
        assert!(s.take_scheduled_failures()[0].contains("(Geen onderwerp)"));
        s.run("draft.schedule", &json!({ "id": id, "send_at": 1 })).unwrap();
        s.run("draft.schedule", &json!({ "id": id })).unwrap();
        assert_eq!(s.draft(&id).unwrap().send_at, None, "without a moment it is a plain draft again");

        // The sample account has nothing to send with: the message is a draft again at once.
        let mut s = Session::sample();
        s.run("draft.save", &json!({ "to": "sanne@voorbeeld.nl", "cc": "", "subject": "Demo", "body": "" })).unwrap();
        let id = s.last_draft.clone().unwrap();
        s.run("draft.schedule", &json!({ "id": id, "send_at": 1 })).unwrap();
        s.send_scheduled(&id);
        assert_eq!(s.draft(&id).map(|d| d.send_at), Some(None));
        assert!(s.scheduled_sends.is_empty() && s.take_scheduled_failures().len() == 1);
    }
}
