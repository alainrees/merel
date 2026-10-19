//! Quick steps, as Outlook has them: a named button that does several things to the selected
//! message at once (move it, mark it read, flag it, give it categories). A step belongs to one
//! account, since the folder it moves to does.

use serde::{Deserialize, Serialize};

use crate::{MAX_CATEGORY_NAME, clean_text};

/// The longest name a quick step may have.
pub const MAX_QUICK_STEP_NAME: usize = 40;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickStep {
    pub id: String,
    pub name: String,
    /// The account whose mail the step works on.
    pub account_id: String,
    /// The folder (session id) the message goes to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<String>,
    #[serde(default)]
    pub mark_read: bool,
    #[serde(default)]
    pub flag: bool,
    /// Categories the message gets, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum QuickStepError {
    #[error("a quick step needs a name")]
    NoName,
    #[error("a quick step needs something to do")]
    NoAction,
    #[error("a quick step belongs to an account")]
    NoAccount,
}

impl QuickStep {
    /// Cleans the name and the categories and checks the step has a name, an action and an
    /// account.
    pub fn validated(mut self) -> Result<Self, QuickStepError> {
        self.name = clean_text(&self.name).trim().chars().take(MAX_QUICK_STEP_NAME).collect();
        if self.name.is_empty() {
            return Err(QuickStepError::NoName);
        }
        let mut categories: Vec<String> = Vec::new();
        for c in &self.categories {
            let c: String = clean_text(c).trim().chars().take(MAX_CATEGORY_NAME).collect();
            if !c.is_empty() && !categories.iter().any(|x| x.eq_ignore_ascii_case(&c)) {
                categories.push(c);
            }
        }
        self.categories = categories;
        if self.move_to.as_deref().is_none_or(str::is_empty) && !self.mark_read && !self.flag && self.categories.is_empty() {
            return Err(QuickStepError::NoAction);
        }
        if self.account_id.is_empty() {
            return Err(QuickStepError::NoAccount);
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_step_needs_a_name_an_action_and_an_account() {
        let step = QuickStep { name: " Klaar \u{7}".into(), account_id: "a".into(), mark_read: true, ..QuickStep::default() };
        assert_eq!(step.clone().validated().unwrap().name, "Klaar");
        assert_eq!(QuickStep { name: " ".into(), ..step.clone() }.validated(), Err(QuickStepError::NoName));
        assert_eq!(QuickStep { mark_read: false, ..step.clone() }.validated(), Err(QuickStepError::NoAction));
        assert_eq!(QuickStep { account_id: String::new(), ..step.clone() }.validated(), Err(QuickStepError::NoAccount));
        let labelled = QuickStep { mark_read: false, categories: vec!["Rood".into(), " rood ".into(), "".into()], ..step };
        assert_eq!(labelled.validated().unwrap().categories, vec!["Rood".to_string()], "a category counts as an action");
    }
}
