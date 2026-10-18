//! Email addresses, validated on the way in.

use serde::{Deserialize, Serialize};

/// Longest address accepted (RFC 5321 path limit).
const MAX_ADDRESS: usize = 254;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("the address is empty")]
    Empty,
    #[error("the address is longer than {MAX_ADDRESS} characters")]
    TooLong,
    #[error("the address needs exactly one @ with text on both sides")]
    Shape,
    #[error("the address contains a character that is not allowed")]
    Character,
}

/// A mailbox: an optional display name and an `local@domain` address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    email: String,
}

impl Address {
    /// Checks the shape of `email` and cleans `name`. This is deliberately strict: no quoted
    /// local parts, comments or whitespace, which are rare in real mail and common in spoofing.
    pub fn new(name: Option<&str>, email: &str) -> Result<Self, AddressError> {
        let email = email.trim();
        if email.is_empty() {
            return Err(AddressError::Empty);
        }
        if email.len() > MAX_ADDRESS {
            return Err(AddressError::TooLong);
        }
        if email.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '<' | '>' | '"' | '(' | ')' | ',' | ';' | '\\')) {
            return Err(AddressError::Character);
        }
        let (local, domain) = email.split_once('@').ok_or(AddressError::Shape)?;
        if local.is_empty() || domain.is_empty() || domain.contains('@') {
            return Err(AddressError::Shape);
        }
        if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
            return Err(AddressError::Shape);
        }
        let name = name.map(crate::clean_text).filter(|n| !n.trim().is_empty());
        Ok(Address { name, email: email.to_string() })
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    /// The name if there is one, otherwise the address.
    pub fn display(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.email)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_addresses() {
        let a = Address::new(Some("Alain"), "alain@example.nl").unwrap();
        assert_eq!(a.email(), "alain@example.nl");
        assert_eq!(a.display(), "Alain");
    }

    #[test]
    fn rejects_injection_and_bad_shapes() {
        assert_eq!(Address::new(None, ""), Err(AddressError::Empty));
        assert_eq!(Address::new(None, "no-at"), Err(AddressError::Shape));
        assert_eq!(Address::new(None, "a@b@c"), Err(AddressError::Shape));
        assert_eq!(Address::new(None, "a@b..nl"), Err(AddressError::Shape));
        assert_eq!(Address::new(None, "a@b.nl\r\nBcc: x@y.nl"), Err(AddressError::Character));
        assert_eq!(Address::new(None, "<a@b.nl>"), Err(AddressError::Character));
        assert_eq!(Address::new(None, &format!("{}@b.nl", "a".repeat(260))), Err(AddressError::TooLong));
    }

    #[test]
    fn blank_name_becomes_none() {
        assert_eq!(Address::new(Some("  "), "a@b.nl").unwrap().display(), "a@b.nl");
    }
}
