//! Blocked and safe senders, as Outlook's junk e-mail options have them. An entry is an
//! address ("news@example.org") or a domain ("example.org", which covers every address there
//! and at its subdomains). Nothing here looks at the message text: the lists are the person's
//! own choices, and the mail server's filter does the rest (with the learning filter of
//! [`crate::Learned`], which learns from those choices). The [`JunkLevel`] says how far the
//! filing goes beyond the blocked list, like Outlook's protection level.

use serde::{Deserialize, Serialize};

/// The longest entry (an address can be 254 characters).
pub const MAX_JUNK_ENTRY: usize = 254;

/// How much new mail is filed as junk on arrival, besides the blocked senders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JunkLevel {
    /// Only the blocked senders (and what the learning filter recognises); the server's filter
    /// does the rest.
    #[default]
    Standard,
    /// Also a sender who shows signs of phishing in the header: a contact's name with another
    /// address, or a look-alike of a known domain.
    High,
    /// Everything that is not from a contact, a safe sender or one of your own addresses.
    SafeOnly,
}

impl JunkLevel {
    pub const ALL: [JunkLevel; 3] = [JunkLevel::Standard, JunkLevel::High, JunkLevel::SafeOnly];

    /// The name a command takes (`junk.level { level }`).
    pub fn id(self) -> &'static str {
        match self {
            JunkLevel::Standard => "standard",
            JunkLevel::High => "high",
            JunkLevel::SafeOnly => "safe_only",
        }
    }

    pub fn from_id(id: &str) -> Option<JunkLevel> {
        JunkLevel::ALL.into_iter().find(|l| l.id() == id)
    }
}

/// What `junk.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JunkLists {
    /// Addresses and domains whose mail goes straight to the Junk folder.
    #[serde(default)]
    pub blocked: Vec<String>,
    /// Addresses and domains that are never filed as junk, whatever `blocked` says.
    #[serde(default)]
    pub safe: Vec<String>,
    #[serde(default)]
    pub level: JunkLevel,
    /// Whether the learning filter (see [`crate::Learned`]) is off; it is on unless turned off.
    #[serde(default, skip_serializing_if = "is_false")]
    pub learning_off: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl JunkLists {
    pub fn is_blocked(&self, email: &str) -> bool {
        self.blocked.iter().any(|e| entry_covers(e, email))
    }

    pub fn is_safe(&self, email: &str) -> bool {
        self.safe.iter().any(|e| entry_covers(e, email))
    }
}

/// The list form of what the person typed: trimmed, lower case, a leading "@" or "*@"
/// dropped. `None` when it is empty, too long, has spaces, or is neither an address (something
/// on both sides of one "@") nor a domain (a dot inside).
pub fn junk_entry(text: &str) -> Option<String> {
    let t = text.trim().trim_start_matches('*').trim_start_matches('@').to_lowercase();
    if t.is_empty() || t.len() > MAX_JUNK_ENTRY || t.chars().any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>') {
        return None;
    }
    match t.split_once('@') {
        Some((local, domain)) => (!local.is_empty() && !domain.is_empty() && !domain.contains('@')).then_some(t),
        None => (t.contains('.') && !t.starts_with('.') && !t.ends_with('.')).then_some(t),
    }
}

/// Whether `entry` (an address or a domain, as [`junk_entry`] gives it) covers `email`: the
/// same address, or its domain or a parent domain of it.
pub fn entry_covers(entry: &str, email: &str) -> bool {
    let email = email.trim().to_lowercase();
    if entry.contains('@') {
        return entry == email;
    }
    let Some((_, domain)) = email.rsplit_once('@') else { return false };
    domain == entry || domain.ends_with(&format!(".{entry}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_cleaned_and_cover_addresses_and_domains() {
        assert_eq!(junk_entry("  News@Example.org "), Some("news@example.org".into()));
        assert_eq!(junk_entry("@example.org"), Some("example.org".into()));
        assert_eq!(junk_entry("*@Example.ORG"), Some("example.org".into()));
        assert_eq!(junk_entry("example.org"), Some("example.org".into()));
        for bad in ["", "   ", "noatsign", "two words.nl", "@", "a@", "@b", "a@b@c", ".example.org", "<x@y.nl>"] {
            assert_eq!(junk_entry(bad), None, "{bad:?}");
        }
        assert!(junk_entry(&"a".repeat(300)).is_none());

        assert!(entry_covers("news@example.org", "NEWS@example.org"));
        assert!(!entry_covers("news@example.org", "other@example.org"));
        assert!(entry_covers("example.org", "x@example.org"));
        assert!(entry_covers("example.org", "x@mail.example.org"), "subdomains count");
        assert!(!entry_covers("example.org", "x@notexample.org"));
        assert!(!entry_covers("example.org", "nonsense"));

        let lists = JunkLists {
            blocked: vec!["spam.example".into()],
            safe: vec!["boss@spam.example".into()],
            level: JunkLevel::Standard,
            learning_off: false,
        };
        assert!(lists.is_blocked("a@spam.example") && lists.is_blocked("boss@spam.example"), "the lists do not know about each other");
        assert!(lists.is_safe("BOSS@spam.example") && !lists.is_safe("a@spam.example"));
    }
}
