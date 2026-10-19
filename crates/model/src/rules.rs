//! Rules for incoming mail: when a message of an account matches, it is moved, marked read or
//! flagged. A rule looks at one part of the envelope for a piece of text, so it can be read
//! back in one line ("Van bevat sanne@").

use serde::{Deserialize, Serialize};

use crate::{MessageHeader, clean_text};

/// The longest text a rule looks for.
pub const MAX_RULE_TEXT: usize = 200;

/// The part of a message a rule looks at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleField {
    /// The sender's name or address.
    #[default]
    From,
    /// Any recipient's name or address.
    To,
    Subject,
    /// Sender, recipients, subject or the preview text.
    Any,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    /// The account whose incoming mail the rule looks at.
    pub account_id: String,
    #[serde(default)]
    pub field: RuleField,
    /// What the field must contain, compared without regard to case.
    pub contains: String,
    /// The folder (session id) a matching message goes to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<String>,
    #[serde(default)]
    pub mark_read: bool,
    #[serde(default)]
    pub flag: bool,
    /// Categories a matching message gets, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RuleError {
    #[error("a rule needs text to look for")]
    NoText,
    #[error("a rule needs something to do")]
    NoAction,
    #[error("a rule belongs to an account")]
    NoAccount,
}

impl Rule {
    /// Cleans the texts and checks the rule has a condition and an action; a nameless rule is
    /// named after its condition.
    pub fn validated(mut self) -> Result<Self, RuleError> {
        self.contains = clean_text(&self.contains).trim().chars().take(MAX_RULE_TEXT).collect();
        self.name = clean_text(&self.name).trim().chars().take(MAX_RULE_TEXT).collect();
        if self.contains.is_empty() {
            return Err(RuleError::NoText);
        }
        let mut categories: Vec<String> = Vec::new();
        for c in &self.categories {
            let c: String = clean_text(c).trim().chars().take(crate::MAX_CATEGORY_NAME).collect();
            if !c.is_empty() && !categories.iter().any(|x| x.eq_ignore_ascii_case(&c)) {
                categories.push(c);
            }
        }
        self.categories = categories;
        if self.move_to.as_deref().is_none_or(str::is_empty) && !self.mark_read && !self.flag && self.categories.is_empty() {
            return Err(RuleError::NoAction);
        }
        if self.account_id.is_empty() {
            return Err(RuleError::NoAccount);
        }
        if self.name.is_empty() {
            self.name = format!("{} bevat {}", self.field.dutch_name(), self.contains);
        }
        Ok(self)
    }

    /// Whether the rule's condition holds for `m`.
    pub fn matches(&self, m: &MessageHeader) -> bool {
        let needle = self.contains.to_lowercase();
        if needle.is_empty() {
            return false;
        }
        let address_text = |a: &crate::Address| format!("{} {}", a.name.as_deref().unwrap_or(""), a.email()).to_lowercase();
        let from = || address_text(&m.from).contains(&needle);
        let to = || m.to.iter().any(|a| address_text(a).contains(&needle));
        let subject = || m.subject.to_lowercase().contains(&needle);
        match self.field {
            RuleField::From => from(),
            RuleField::To => to(),
            RuleField::Subject => subject(),
            RuleField::Any => from() || to() || subject() || m.preview.to_lowercase().contains(&needle),
        }
    }
}

impl RuleField {
    pub const ALL: [RuleField; 4] = [RuleField::From, RuleField::To, RuleField::Subject, RuleField::Any];

    /// The label the window shows: "Van", "Aan", "Onderwerp", "Bericht".
    pub fn dutch_name(self) -> &'static str {
        match self {
            RuleField::From => "Van",
            RuleField::To => "Aan",
            RuleField::Subject => "Onderwerp",
            RuleField::Any => "Bericht",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Address, Flags};

    fn message() -> MessageHeader {
        MessageHeader {
            id: "m".into(),
            folder_id: "a/INBOX".into(),
            remote_id: "1".into(),
            from: Address::new(Some("Sanne de Vries"), "sanne@example.org").unwrap(),
            to: vec![Address::new(None, "team@example.org").unwrap()],
            subject: "Factuur oktober".into(),
            date: 0,
            preview: "Bijgevoegd de factuur".into(),
            flags: Flags::default(),
            has_attachments: true,
            message_id: None,
            categories: Vec::new(),
        }
    }

    fn rule(field: RuleField, contains: &str) -> Rule {
        Rule { account_id: "a".into(), field, contains: contains.into(), mark_read: true, ..Rule::default() }
    }

    #[test]
    fn rules_match_the_part_they_name_without_regard_to_case() {
        let m = message();
        assert!(rule(RuleField::From, "SANNE@").matches(&m));
        assert!(rule(RuleField::From, "de vries").matches(&m));
        assert!(!rule(RuleField::From, "team@").matches(&m));
        assert!(rule(RuleField::To, "team@").matches(&m));
        assert!(rule(RuleField::Subject, "factuur").matches(&m));
        assert!(!rule(RuleField::Subject, "bijgevoegd").matches(&m));
        assert!(rule(RuleField::Any, "bijgevoegd").matches(&m));
        assert!(!rule(RuleField::Any, "").matches(&m));
    }

    #[test]
    fn validation_wants_text_an_action_and_an_account_and_names_the_rule() {
        let ok = rule(RuleField::Subject, "  Factuur \u{7} ").validated().unwrap();
        assert_eq!((ok.contains.as_str(), ok.name.as_str()), ("Factuur", "Onderwerp bevat Factuur"));
        assert_eq!(rule(RuleField::From, " ").validated(), Err(RuleError::NoText));
        assert_eq!(Rule { mark_read: false, ..rule(RuleField::From, "x") }.validated(), Err(RuleError::NoAction));
        assert_eq!(Rule { account_id: String::new(), ..rule(RuleField::From, "x") }.validated(), Err(RuleError::NoAccount));
        let moved = Rule { mark_read: false, move_to: Some("a/Archive".into()), name: "Archief".into(), ..rule(RuleField::From, "x") };
        assert_eq!(moved.validated().unwrap().name, "Archief");
        let labelled = Rule { mark_read: false, categories: vec![" Rood ".into(), "rood".into(), " ".into()], ..rule(RuleField::From, "x") };
        assert_eq!(labelled.validated().unwrap().categories, vec!["Rood".to_string()], "a category is an action too, cleaned and listed once");
    }
}
