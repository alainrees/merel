//! How the message list is shown: which messages (a [`Filter`], and a category with
//! `view.category { name? }`) and in what order (a [`Sort`]), set with `view.filter { filter }`
//! and `view.sort { sort }`. The search box narrows the list further; all of it leaves the
//! folder itself alone. [`conversations`] groups the list by subject for the conversation view.

use std::cmp::Ordering;
use std::collections::HashMap;

use merel_model::{MessageHeader, conversation_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Session;
use crate::commands::CommandError;

/// Which messages of the folder the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    #[default]
    All,
    Unread,
    Flagged,
    Attachments,
    /// Only the messages put off with "Sluimeren"; every other filter leaves them out.
    Snoozed,
}

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::All, Filter::Unread, Filter::Flagged, Filter::Attachments, Filter::Snoozed];

    pub fn dutch_name(self) -> &'static str {
        match self {
            Filter::All => "Alles",
            Filter::Unread => "Ongelezen",
            Filter::Flagged => "Gemarkeerd",
            Filter::Attachments => "Met bijlage",
            Filter::Snoozed => "Gesluimerd",
        }
    }

    pub fn keeps(self, m: &MessageHeader) -> bool {
        match self {
            Filter::All => true,
            Filter::Unread => !m.flags.seen,
            Filter::Flagged => m.flags.flagged,
            Filter::Attachments => m.has_attachments,
            // Snoozed or not is the session's to know; see `Session::visible_messages`.
            Filter::Snoozed => true,
        }
    }
}

/// The order of the list; a tie is broken by date, newest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    #[default]
    Newest,
    Oldest,
    Sender,
    Subject,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Newest, Sort::Oldest, Sort::Sender, Sort::Subject];

    pub fn dutch_name(self) -> &'static str {
        match self {
            Sort::Newest => "Nieuwste eerst",
            Sort::Oldest => "Oudste eerst",
            Sort::Sender => "Op afzender",
            Sort::Subject => "Op onderwerp",
        }
    }

    pub fn order(self, a: &MessageHeader, b: &MessageHeader) -> Ordering {
        let newest_first = b.date.cmp(&a.date);
        match self {
            Sort::Newest => newest_first,
            Sort::Oldest => a.date.cmp(&b.date),
            Sort::Sender => a.from.display().to_lowercase().cmp(&b.from.display().to_lowercase()).then(newest_first),
            Sort::Subject => a.subject.to_lowercase().cmp(&b.subject.to_lowercase()).then(newest_first),
        }
    }
}

/// The messages on one subject, in the order of the list; the first is the one a folded
/// conversation shows.
#[derive(Debug)]
pub struct Conversation<'a> {
    /// [`conversation_key`] of the subject.
    pub key: String,
    pub messages: Vec<&'a MessageHeader>,
}

impl Conversation<'_> {
    pub fn unread(&self) -> usize {
        self.messages.iter().filter(|m| !m.flags.seen).count()
    }
}

/// Groups a list by [`conversation_key`]; each conversation stands where its first message
/// stood, with its messages in the list's order.
pub fn conversations(list: Vec<&MessageHeader>) -> Vec<Conversation<'_>> {
    let mut out: Vec<Conversation<'_>> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for m in list {
        let key = conversation_key(&m.subject);
        match index.get(&key) {
            Some(&i) => out[i].messages.push(m),
            None => {
                index.insert(key.clone(), out.len());
                out.push(Conversation { key, messages: vec![m] });
            }
        }
    }
    out
}

impl Session {
    /// [`Self::visible_messages`] grouped by subject; see [`conversations`].
    pub fn visible_conversations(&self) -> Vec<Conversation<'_>> {
        conversations(self.visible_messages())
    }
}

/// `view.filter { filter }`: "all", "unread", "flagged" or "attachments".
pub fn filter(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    s.filter = serde_json::from_value(params.get("filter").cloned().unwrap_or(Value::Null)).map_err(|_| CommandError::Param("filter"))?;
    Ok(())
}

/// `view.sort { sort }`: "newest", "oldest", "sender" or "subject".
pub fn sort(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    s.sort = serde_json::from_value(params.get("sort").cloned().unwrap_or(Value::Null)).map_err(|_| CommandError::Param("sort"))?;
    Ok(())
}

/// `view.category { name? }`: only messages that carry the category; no name (or an empty one)
/// shows them all again. The name need not be in the person's list: one from the server works.
pub fn category(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    s.category_filter = match params.get("name") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or(CommandError::Param("name"))?.trim()).filter(|n| !n.is_empty()).map(str::to_string),
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn subjects(s: &Session) -> Vec<String> {
        s.visible_messages().iter().map(|m| m.subject.clone()).collect()
    }

    #[test]
    fn conversations_gather_replies_behind_the_message_that_stands_first() {
        let mut s = Session::sample();
        let mut earlier = s.message("m2").expect("Re: Offerte drukwerk").clone();
        earlier.id = "m9".into();
        earlier.subject = "Offerte drukwerk".into();
        earlier.date -= 86_400;
        earlier.flags.seen = false;
        s.messages.push(earlier);
        let groups = s.visible_conversations();
        assert_eq!(groups.len(), 4, "five messages, four subjects");
        let g = groups.iter().find(|g| g.key == "offerte drukwerk").expect("the group");
        assert_eq!(g.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m2", "m9"], "newest first, as the list");
        assert_eq!(g.unread(), 1);
        s.run("view.sort", &json!({ "sort": "oldest" })).expect("sort");
        let groups = s.visible_conversations();
        assert_eq!(
            groups[0].messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["m9", "m2"],
            "the group stands where its first message stands"
        );
        s.run("view.filter", &json!({ "filter": "unread" })).expect("filter");
        assert!(s.visible_conversations().iter().all(|g| g.messages.iter().all(|m| !m.flags.seen)), "the filter applies before grouping");
    }

    #[test]
    fn the_list_is_filtered_and_sorted_as_asked() {
        let mut s = Session::sample();
        assert_eq!(subjects(&s)[0], "Welkom bij Merel", "newest first by default");
        s.run("view.filter", &json!({ "filter": "attachments" })).expect("filter");
        assert_eq!(subjects(&s), vec!["Planning voor donderdag"]);
        s.run("view.filter", &json!({ "filter": "unread" })).expect("filter");
        assert_eq!(subjects(&s), vec!["Welkom bij Merel", "Planning voor donderdag"]);
        s.run("view.filter", &json!({ "filter": "all" })).expect("filter");
        s.run("view.sort", &json!({ "sort": "sender" })).expect("sort");
        assert_eq!(subjects(&s), vec!["Wat er deze maand speelt", "Re: Offerte drukwerk", "Planning voor donderdag", "Welkom bij Merel"]);
        s.run("view.sort", &json!({ "sort": "oldest" })).expect("sort");
        assert_eq!(subjects(&s).last().map(String::as_str), Some("Welkom bij Merel"));
        assert!(matches!(s.run("view.sort", &json!({ "sort": "sideways" })), Err(CommandError::Param("sort"))));
        s.run("view.category", &json!({ "name": "groene categorie" })).expect("category");
        assert_eq!(subjects(&s), vec!["Planning voor donderdag"]);
        s.run("view.category", &json!({ "name": "" })).expect("all again");
        assert_eq!(subjects(&s).len(), 4);
        assert!(matches!(s.run("view.category", &json!({ "name": 3 })), Err(CommandError::Param("name"))));
    }
}
