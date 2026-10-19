//! How the message list is shown: which messages (a [`Filter`]) and in what order (a [`Sort`]),
//! set with `view.filter { filter }` and `view.sort { sort }`. The search box narrows the list
//! further; both leave the folder itself alone.

use std::cmp::Ordering;

use mailcraft_model::MessageHeader;
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
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Unread, Filter::Flagged, Filter::Attachments];

    pub fn dutch_name(self) -> &'static str {
        match self {
            Filter::All => "Alles",
            Filter::Unread => "Ongelezen",
            Filter::Flagged => "Gemarkeerd",
            Filter::Attachments => "Met bijlage",
        }
    }

    pub fn keeps(self, m: &MessageHeader) -> bool {
        match self {
            Filter::All => true,
            Filter::Unread => !m.flags.seen,
            Filter::Flagged => m.flags.flagged,
            Filter::Attachments => m.has_attachments,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn subjects(s: &Session) -> Vec<String> {
        s.visible_messages().iter().map(|m| m.subject.clone()).collect()
    }

    #[test]
    fn the_list_is_filtered_and_sorted_as_asked() {
        let mut s = Session::sample();
        assert_eq!(subjects(&s)[0], "Welkom bij Mailcraft", "newest first by default");
        s.run("view.filter", &json!({ "filter": "attachments" })).expect("filter");
        assert_eq!(subjects(&s), vec!["Planning voor donderdag"]);
        s.run("view.filter", &json!({ "filter": "unread" })).expect("filter");
        assert_eq!(subjects(&s), vec!["Welkom bij Mailcraft", "Planning voor donderdag"]);
        s.run("view.filter", &json!({ "filter": "all" })).expect("filter");
        s.run("view.sort", &json!({ "sort": "sender" })).expect("sort");
        assert_eq!(subjects(&s), vec!["Wat er deze maand speelt", "Re: Offerte drukwerk", "Planning voor donderdag", "Welkom bij Mailcraft"]);
        s.run("view.sort", &json!({ "sort": "oldest" })).expect("sort");
        assert_eq!(subjects(&s).last().map(String::as_str), Some("Welkom bij Mailcraft"));
        assert!(matches!(s.run("view.sort", &json!({ "sort": "sideways" })), Err(CommandError::Param("sort"))));
    }
}
