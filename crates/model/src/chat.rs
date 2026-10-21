//! Chat over e-mail, the way Delta Chat does it (chatmail): a chat message is an ordinary,
//! end-to-end encrypted message with a `Chat-Version` header, so any mail account carries it
//! and the other side needs no Merel. A group chat names itself with `Chat-Group-ID` and
//! `Chat-Group-Name`.

use serde::{Deserialize, Serialize};

use crate::clean_text;

/// The `Chat-Version` Merel writes.
pub const CHAT_VERSION: &str = "1.0";
/// Most characters a group id may have; Delta Chat makes them 11 to 22.
pub const MAX_GROUP_ID: usize = 64;
/// Most characters a group name keeps.
pub const MAX_GROUP_NAME: usize = 200;

/// What makes a message a chat message, from its headers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatTag {
    /// The group the message belongs to; `None` for a chat between two people.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
}

impl ChatTag {
    /// The tag a message with these header values carries; `None` without `Chat-Version`. A
    /// group id is only taken when it is a plain token, so it can be used as a key.
    pub fn from_headers(version: Option<&str>, group_id: Option<&str>, group_name: Option<&str>) -> Option<ChatTag> {
        version.filter(|v| !v.trim().is_empty())?;
        let group_id = group_id
            .map(str::trim)
            .filter(|id| !id.is_empty() && id.len() <= MAX_GROUP_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
            .map(str::to_string);
        let group_name = group_id
            .as_ref()
            .and(group_name)
            .map(|n| clean_text(n.trim()).chars().take(MAX_GROUP_NAME).collect::<String>())
            .filter(|n| !n.is_empty());
        Some(ChatTag { group_id, group_name })
    }

    /// A new group id: 16 characters from `random`, of the alphabet Delta Chat uses.
    pub fn new_group_id(random: [u8; 16]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        random.iter().map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_comes_from_chat_version_and_keeps_only_a_clean_group() {
        assert_eq!(ChatTag::from_headers(None, Some("abc"), Some("x")), None);
        assert_eq!(ChatTag::from_headers(Some(" "), None, None), None);
        assert_eq!(ChatTag::from_headers(Some("1.0"), None, Some("Naam zonder groep")), Some(ChatTag::default()));
        let group = ChatTag::from_headers(Some("1.0"), Some(" Ab-9_x "), Some(" Familie\u{7} Rees ")).unwrap();
        assert_eq!(group.group_id.as_deref(), Some("Ab-9_x"));
        assert_eq!(group.group_name.as_deref(), Some("Familie Rees"));
        assert_eq!(ChatTag::from_headers(Some("1.0"), Some("a/b"), Some("x")), Some(ChatTag::default()), "not a token");
        assert_eq!(ChatTag::from_headers(Some("1.0"), Some(&"a".repeat(65)), None), Some(ChatTag::default()));
    }

    #[test]
    fn a_new_group_id_is_a_token() {
        let id = ChatTag::new_group_id([255; 16]);
        assert_eq!(id.len(), 16);
        assert!(ChatTag::from_headers(Some("1.0"), Some(&id), None).unwrap().group_id.is_some());
    }
}
