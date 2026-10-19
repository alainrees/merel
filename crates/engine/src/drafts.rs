//! Messages put aside while being written. They live in the session and, per account, in the
//! encrypted offline cache, so a half-written mail survives a restart without ever reaching a
//! plain file. Nothing goes to the server: a draft is this computer's until it is sent.

use serde_json::Value;

use mailcraft_model::StoredDraft;

use crate::{CommandError, SAMPLE_ACCOUNT, Session};

fn now() -> i64 {
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
}
