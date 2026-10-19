//! Text templates, as Outlook's "Mijn sjablonen": a named piece of text to drop into a message
//! with one click, kept on this computer only.

use serde::{Deserialize, Serialize};

use crate::clean_text;

/// The longest name a template may have.
pub const MAX_TEMPLATE_NAME: usize = 60;
/// The most characters a template's text may have.
pub const MAX_TEMPLATE_TEXT: usize = 20_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("a template needs a name")]
    NoName,
    #[error("a template needs text")]
    NoText,
    #[error("a template holds at most {MAX_TEMPLATE_TEXT} characters")]
    TooLong,
}

impl Template {
    /// Cleans the name, keeps the text's line breaks and tabs but no other control characters,
    /// and checks the template has a name and text.
    pub fn validated(mut self) -> Result<Self, TemplateError> {
        self.name = clean_text(&self.name).trim().chars().take(MAX_TEMPLATE_NAME).collect();
        if self.name.is_empty() {
            return Err(TemplateError::NoName);
        }
        let text: String = self.text.replace("\r\n", "\n").chars().filter(|c| !c.is_control() || matches!(c, '\t' | '\n')).collect();
        self.text = text.trim_matches(['\n', ' ', '\t']).to_string();
        if self.text.is_empty() {
            return Err(TemplateError::NoText);
        }
        if self.text.chars().count() > MAX_TEMPLATE_TEXT {
            return Err(TemplateError::TooLong);
        }
        Ok(self)
    }

    /// The first line of the text, for a list.
    pub fn first_line(&self) -> &str {
        self.text.lines().next().unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_needs_a_name_and_text_and_keeps_its_line_breaks() {
        let t = Template { id: "t1".into(), name: " Routebeschrijving \u{7}".into(), text: "Kom via de A2.\r\n\tAfslag 9.\u{8}\n\n".into() };
        let ok = t.clone().validated().unwrap();
        assert_eq!(ok.name, "Routebeschrijving");
        assert_eq!(ok.text, "Kom via de A2.\n\tAfslag 9.");
        assert_eq!(ok.first_line(), "Kom via de A2.");
        assert_eq!(Template { name: "  ".into(), ..t.clone() }.validated(), Err(TemplateError::NoName));
        assert_eq!(Template { text: "\n \n".into(), ..t.clone() }.validated(), Err(TemplateError::NoText));
        assert_eq!(Template { text: "x".repeat(MAX_TEMPLATE_TEXT + 1), ..t }.validated(), Err(TemplateError::TooLong));
    }
}
