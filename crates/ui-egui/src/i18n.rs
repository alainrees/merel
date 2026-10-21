//! The words on screen in the person's language. Each language is one Fluent file under
//! `crates/ui-egui/locales` (`nl.ftl`, `en.ftl`), built into the program; [`tr`] looks a key up
//! in the language in use and falls back to English, then to the key itself, so a missing
//! translation shows something readable rather than nothing.
//!
//! The language in use is one global setting, as the person picks one for the whole program
//! under Beeld > Taal (or lets the computer decide). Tests always run in Dutch, whatever the
//! machine they run on is set to, and each test thread can switch on its own with [`set`].

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use serde::{Deserialize, Serialize};
use unic_langid::LanguageIdentifier;

/// A language Merel speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Dutch,
    English,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::Dutch, Language::English];

    /// The language's name in that language itself, as a language menu lists them, so someone
    /// who cannot read the current one still finds their own.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::Dutch => "Nederlands",
            Language::English => "English",
        }
    }

    /// The BCP 47 tag, for Fluent's plural rules and for the date formats.
    pub fn tag(self) -> &'static str {
        match self {
            Language::Dutch => "nl",
            Language::English => "en",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Language::Dutch => include_str!("../locales/nl.ftl"),
            Language::English => include_str!("../locales/en.ftl"),
        }
    }

    /// The language for a locale such as `nl-NL` or `en_GB.UTF-8`: Dutch for Dutch, English
    /// for every other, since English is the one most people read after their own.
    pub fn for_locale(locale: &str) -> Language {
        let lang = locale.split(['-', '_', '.', '@']).next().unwrap_or_default();
        if lang.eq_ignore_ascii_case("nl") { Language::Dutch } else { Language::English }
    }

    /// The language the computer is set to (macOS, Windows or Linux).
    pub fn of_system() -> Language {
        sys_locale::get_locale().map_or(Language::English, |l| Language::for_locale(&l))
    }

    fn code(self) -> u8 {
        match self {
            Language::Dutch => 0,
            Language::English => 1,
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    fn from_code(code: u8) -> Language {
        if code == 1 { Language::English } else { Language::Dutch }
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

static CURRENT: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    // Tests run side by side on threads; each keeps its own language so one test in English
    // does not turn its neighbours English.
    static TEST_LANGUAGE: std::cell::Cell<Language> = const { std::cell::Cell::new(Language::Dutch) };
}

/// The language the words on screen are in now.
pub fn current() -> Language {
    #[cfg(test)]
    {
        TEST_LANGUAGE.with(|l| l.get())
    }
    #[cfg(not(test))]
    {
        Language::from_code(CURRENT.load(Ordering::Relaxed))
    }
}

/// Switches the words on screen to `language`, from the next frame on.
pub fn set(language: Language) {
    #[cfg(test)]
    TEST_LANGUAGE.with(|l| l.set(language));
    CURRENT.store(language.code(), Ordering::Relaxed);
}

fn bundle(language: Language) -> FluentBundle<FluentResource> {
    let id: LanguageIdentifier = language.tag().parse().unwrap_or_default();
    let mut bundle = FluentBundle::new_concurrent(vec![id]);
    // No invisible direction marks around { $values }: both languages run left to right, and
    // the marks would show as boxes in fonts without them.
    bundle.set_use_isolating(false);
    // A file that does not parse still gives the messages that do; the test below keeps the
    // files clean, so this only matters for a hand-edited translation.
    let resource = FluentResource::try_new(language.source().to_string()).unwrap_or_else(|(resource, _)| resource);
    if let Err(errors) = bundle.add_resource(resource) {
        log::warn!("{} translation: {errors:?}", language.tag());
    }
    bundle
}

static BUNDLES: LazyLock<[FluentBundle<FluentResource>; 2]> = LazyLock::new(|| [bundle(Language::Dutch), bundle(Language::English)]);

fn format(language: Language, key: &str, args: Option<&FluentArgs>) -> Option<String> {
    let bundle = &BUNDLES[usize::from(language.code())];
    let pattern = bundle.get_message(key)?.value()?;
    let mut errors = Vec::new();
    let text = bundle.format_pattern(pattern, args, &mut errors);
    if !errors.is_empty() {
        log::warn!("{key} ({}): {errors:?}", language.tag());
    }
    Some(text.into_owned())
}

/// The text for `key` when it needs no arguments.
fn plain(language: Language, key: &str) -> Option<String> {
    let bundle = &BUNDLES[usize::from(language.code())];
    let mut errors = Vec::new();
    let text = bundle.format_pattern(bundle.get_message(key)?.value()?, None, &mut errors);
    errors.is_empty().then(|| text.into_owned())
}

fn lookup(key: &str, args: Option<&FluentArgs>) -> String {
    format(current(), key, args).or_else(|| format(Language::English, key, args)).unwrap_or_else(|| key.to_string())
}

/// The text for `key` in the current language.
pub fn tr(key: &str) -> String {
    lookup(key, None)
}

/// The text for `key` with its `{ $name }` placeholders filled in, such as a count that picks
/// "1 bericht" or "3 berichten".
pub fn tr_args<'a>(key: &str, args: &[(&'a str, FluentValue<'a>)]) -> String {
    let mut fluent = FluentArgs::new();
    for (name, value) in args {
        fluent.set(*name, value.clone());
    }
    lookup(key, Some(&fluent))
}

/// The Dutch words for a text shown in the current language, for the tables that still pick an
/// icon or a style by a button's Dutch name; other text comes back as it is.
pub fn dutch(text: &str) -> std::borrow::Cow<'_, str> {
    static TO_DUTCH: LazyLock<std::collections::HashMap<String, String>> = LazyLock::new(|| {
        let mut map = std::collections::HashMap::new();
        if let Ok(resource) = FluentResource::try_new(Language::English.source().to_string()) {
            for entry in resource.entries() {
                if let fluent_syntax::ast::Entry::Message(m) = entry
                    && let (Some(english), Some(dutch)) = (plain(Language::English, m.id.name), plain(Language::Dutch, m.id.name))
                {
                    map.entry(english).or_insert(dutch);
                }
            }
        }
        map
    });
    match current() {
        Language::Dutch => std::borrow::Cow::Borrowed(text),
        _ => TO_DUTCH.get(text).map_or(std::borrow::Cow::Borrowed(text), |d| std::borrow::Cow::Owned(d.clone())),
    }
}

/// The Dutch texts that differ from their English, for the tests that look for Dutch left on an
/// English screen.
#[cfg(test)]
pub(crate) fn dutch_only_texts() -> std::collections::HashSet<String> {
    let Ok(resource) = FluentResource::try_new(Language::Dutch.source().to_string()) else { return Default::default() };
    resource
        .entries()
        .filter_map(|e| match e {
            fluent_syntax::ast::Entry::Message(m) => Some(m.id.name),
            _ => None,
        })
        .filter_map(|key| Some((plain(Language::Dutch, key)?, plain(Language::English, key)?)))
        .filter(|(nl, en)| nl != en)
        .map(|(nl, _)| nl)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn keys(language: Language) -> BTreeSet<String> {
        let resource = FluentResource::try_new(language.source().to_string())
            .unwrap_or_else(|(_, errors)| panic!("{} does not parse: {errors:?}", language.tag()));
        resource
            .entries()
            .filter_map(|e| match e {
                fluent_syntax::ast::Entry::Message(m) => Some(m.id.name.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_language_has_every_text() {
        let dutch = keys(Language::Dutch);
        for language in Language::ALL {
            let theirs = keys(language);
            assert!(!theirs.is_empty());
            let missing: Vec<_> = dutch.difference(&theirs).collect();
            let extra: Vec<_> = theirs.difference(&dutch).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{}: missing {missing:?}, not in Dutch {extra:?}", language.tag());
        }
    }

    #[test]
    fn texts_follow_the_language_and_fall_back_to_english_then_the_key() {
        assert_eq!(tr("theme-dark"), "Donker", "tests start in Dutch");
        set(Language::English);
        assert_eq!(tr("theme-dark"), "Dark");
        assert_eq!(LanguageChoice::System.name(), "System language");
        assert_eq!(LanguageChoice::Dutch.name(), "Nederlands", "a language keeps its own name");
        set(Language::Dutch);
        assert_eq!(tr("no-such-text"), "no-such-text");
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

    #[test]
    fn the_computers_locale_picks_dutch_or_else_english() {
        for (locale, language) in [
            ("nl-NL", Language::Dutch),
            ("nl_BE.UTF-8", Language::Dutch),
            ("NL", Language::Dutch),
            ("en-US", Language::English),
            ("de-DE", Language::English),
            ("C", Language::English),
            ("", Language::English),
        ] {
            assert_eq!(Language::for_locale(locale), language, "{locale}");
        }
        assert_eq!(LanguageChoice::System.language(), Language::Dutch, "tests treat the computer as Dutch");
    }
}
