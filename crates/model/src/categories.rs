//! Categories: the coloured labels Outlook puts on mail. A message carries category names
//! ([`super::MessageHeader::categories`]); the list of known categories with their colours is
//! the person's own and lives on this computer. Servers keep the names: Exchange and Microsoft
//! 365 as the item's categories, IMAP as keywords (custom flags).

use serde::{Deserialize, Serialize};

/// The longest category name accepted, in characters.
pub const MAX_CATEGORY_NAME: usize = 50;

/// Outlook's palette, as far as it goes here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryColor {
    Red,
    Orange,
    Yellow,
    Green,
    #[default]
    Blue,
    Purple,
    Gray,
}

impl CategoryColor {
    pub const ALL: [CategoryColor; 7] = [
        CategoryColor::Red,
        CategoryColor::Orange,
        CategoryColor::Yellow,
        CategoryColor::Green,
        CategoryColor::Blue,
        CategoryColor::Purple,
        CategoryColor::Gray,
    ];

    pub fn dutch_name(self) -> &'static str {
        match self {
            CategoryColor::Red => "Rood",
            CategoryColor::Orange => "Oranje",
            CategoryColor::Yellow => "Geel",
            CategoryColor::Green => "Groen",
            CategoryColor::Blue => "Blauw",
            CategoryColor::Purple => "Paars",
            CategoryColor::Gray => "Grijs",
        }
    }
}

/// A category as the person knows it: a name and a colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub color: CategoryColor,
}

impl Category {
    /// Outlook's six starting categories, in Dutch.
    pub fn defaults() -> Vec<Category> {
        [
            ("Rode categorie", CategoryColor::Red),
            ("Oranje categorie", CategoryColor::Orange),
            ("Gele categorie", CategoryColor::Yellow),
            ("Groene categorie", CategoryColor::Green),
            ("Blauwe categorie", CategoryColor::Blue),
            ("Paarse categorie", CategoryColor::Purple),
        ]
        .into_iter()
        .map(|(name, color)| Category { name: name.into(), color })
        .collect()
    }
}

/// The IMAP keyword for a category name: an atom, so letters, digits, `-` and `.` stay and
/// everything else (spaces included) becomes `_`. Read back, `_` becomes a space again.
pub fn category_keyword(name: &str) -> String {
    name.trim().chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '.') { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_are_atoms() {
        assert_eq!(category_keyword(" Rode categorie "), "Rode_categorie");
        assert_eq!(category_keyword("Privé (thuis)"), "Priv___thuis_");
        assert_eq!(Category::defaults().len(), 6);
    }
}
