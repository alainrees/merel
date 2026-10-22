//! The words on screen in the person's language: the Fluent files and lookups of
//! `merel-i18n`, and the choice under Beeld > Taal. Tests always run in Dutch, whatever the
//! machine they run on is set to, and each test thread can switch on its own with [`set`].

use serde::{Deserialize, Serialize};

pub use merel_i18n::{
    FluentValue, Language, current, dutch, month_name, month_short, plain, tr, tr_args, weekday_letter, weekday_name, weekday_short,
};

#[cfg(test)]
pub(crate) use merel_i18n::dutch_only_texts;

/// Switches the words on screen to `language`, from the next frame on; in a test, only on the
/// test's own thread, so a test in English does not turn its neighbours English.
pub fn set(language: Language) {
    if cfg!(test) {
        merel_i18n::set_for_thread(language);
    } else {
        merel_i18n::set(language);
    }
}

/// The person's choice under Beeld > Taal: the computer's language, or one of Merel's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageChoice {
    #[default]
    System,
    Dutch,
    English,
}

impl LanguageChoice {
    pub const ALL: [LanguageChoice; 3] = [LanguageChoice::System, LanguageChoice::Dutch, LanguageChoice::English];

    /// The language this choice comes to on this computer. Tests treat the computer as Dutch.
    pub fn language(self) -> Language {
        match self {
            LanguageChoice::System if cfg!(test) => Language::Dutch,
            LanguageChoice::System => Language::of_system(),
            LanguageChoice::Dutch => Language::Dutch,
            LanguageChoice::English => Language::English,
        }
    }

    /// The button's name: "Systeem" in the current language, the languages in their own.
    pub fn name(self) -> String {
        match self {
            LanguageChoice::System => tr("language-system"),
            LanguageChoice::Dutch => Language::Dutch.native_name().to_string(),
            LanguageChoice::English => Language::English.native_name().to_string(),
        }
    }

    pub fn hint(self) -> String {
        tr(match self {
            LanguageChoice::System => "language-system-hint",
            LanguageChoice::Dutch => "language-dutch-hint",
            LanguageChoice::English => "language-english-hint",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_are_named_in_the_current_language() {
        set(Language::English);
        assert_eq!(LanguageChoice::System.name(), "System language");
        assert_eq!(LanguageChoice::Dutch.name(), "Nederlands", "a language keeps its own name");
        set(Language::Dutch);
        assert_eq!(LanguageChoice::System.language(), Language::Dutch, "tests treat the computer as Dutch");
    }

    #[test]
    fn ribbon_buttons_keep_their_icons_in_english() {
        use crate::helder::{Icon, ribbon_icon, ribbon_icon_only, ribbon_primary};
        set(Language::English);
        assert_eq!(ribbon_icon(&tr("mail-reply"), "↩"), Some(Icon::Reply));
        assert_eq!(ribbon_icon(&tr("calendar-week"), "▥"), Some(Icon::CalendarRange));
        assert!(ribbon_icon_only(&tr("delete")) && ribbon_icon_only(&tr("theme-dark")));
        assert!(ribbon_primary(&tr("new-mail")) && !ribbon_primary(&tr("mail-reply")));
        assert_eq!(dutch("Reply"), "Beantwoorden");
        set(Language::Dutch);
    }

    #[test]
    fn the_engines_choices_have_their_words() {
        for language in Language::ALL {
            set(language);
            for (when, _) in merel_engine::followup::WHEN {
                let key = format!("followup-when-{when}");
                assert_ne!(tr(&key), key);
            }
            for (when, _) in merel_engine::snooze::WHEN {
                let key = format!("snooze-{when}");
                assert_ne!(tr(&key), key);
            }
        }
        set(Language::Dutch);
    }
}
