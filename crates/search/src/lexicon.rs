//! Words that mean the same in Dutch, English, German and French, so a search for "voorstel"
//! also finds the mail that says "quote", "Angebot" or "devis". The list ([`lexicon.txt`]) is
//! built in: the words of mail, work and daily life, some thousands, each line one meaning. A
//! word is found by its folded form (lower case, no accents) or by its stem in any of the four
//! languages, so "facturen" and "Rechnungen" find their line too. A word written "=word" in the
//! list is only found as written, where its stem would tie it to a line about something else.

use std::sync::OnceLock;

use tantivy::tokenizer::{Language, TextAnalyzer};

use super::{fold, stemmed, tokens};

const LIST: &str = include_str!("lexicon.txt");

/// Shorter words are left alone: too many of them mean something else somewhere.
const SHORTEST: usize = 3;

/// Shorter stems tie words of different languages that have nothing to do with each other
/// ("vat" and "Vater", "wife" and "wifi"), so two words only share a stem this long.
const SHORTEST_STEM: usize = 5;

/// One word of the list: its line, how it is written there, its forms, and whether it is only
/// found as written.
struct Entry {
    line: usize,
    word: &'static str,
    forms: Forms,
    exact: bool,
}

/// A word folded and stemmed in each of the four languages.
#[derive(PartialEq, Eq)]
struct Forms {
    folded: String,
    stems: [String; 4],
}

struct Analyzers {
    fold: TextAnalyzer,
    stems: [TextAnalyzer; 4],
}

impl Analyzers {
    fn new() -> Analyzers {
        Analyzers { fold: fold(), stems: [Language::Dutch, Language::English, Language::German, Language::French].map(stemmed) }
    }

    /// The forms of a single word; `None` for nothing, or for more than one word.
    fn forms(&mut self, word: &str) -> Option<Forms> {
        let mut folded = tokens(&mut self.fold, word).into_iter();
        let (folded, _) = folded.next().filter(|_| folded.next().is_none())?;
        let stems = std::array::from_fn(|i| tokens(&mut self.stems[i], &folded).into_iter().next().map(|t| t.0).unwrap_or_else(|| folded.clone()));
        Some(Forms { folded, stems })
    }
}

fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let mut a = Analyzers::new();
        let lines = LIST.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
        lines
            .enumerate()
            .flat_map(|(line, words)| words.split(',').map(move |w| (line, w.trim())))
            .filter_map(|(line, word)| {
                let (word, exact) = word.strip_prefix('=').map_or((word, false), |w| (w, true));
                Some(Entry { line, word, forms: a.forms(word)?, exact })
            })
            .collect()
    })
}

/// Whether `word` is this word of the list: written alike, or with the same stem in the same
/// language, unless the list wants it as written.
fn same(e: &Entry, word: &Forms) -> bool {
    e.forms.folded == word.folded || !e.exact && e.forms.stems.iter().zip(&word.stems).any(|(x, y)| x.chars().count() >= SHORTEST_STEM && x == y)
}

/// The words that mean what `word` means, in the four languages, in the list's order; empty
/// when the list does not know it. The word itself is not among them.
pub fn alike(word: &str) -> Vec<String> {
    let mut a = Analyzers::new();
    let Some(forms) = a.forms(word).filter(|f| f.folded.chars().count() >= SHORTEST && !f.folded.chars().all(|c| c.is_ascii_digit())) else {
        return Vec::new();
    };
    let entries = entries();
    let lines: Vec<usize> = entries.iter().filter(|e| same(e, &forms)).map(|e| e.line).collect();
    let mut out: Vec<String> = Vec::new();
    for e in entries.iter().filter(|e| lines.contains(&e.line)) {
        if !same(e, &forms) && !out.iter().any(|w| w == e.word) {
            out.push(e.word.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_finds_its_meaning_in_four_languages() {
        let offer = alike("voorstel");
        for w in ["offerte", "quote", "proposal", "angebot", "devis"] {
            assert!(offer.contains(&w.to_string()), "{w} in {offer:?}");
        }
        assert!(!offer.contains(&"voorstel".to_string()), "not the word itself");
        assert!(alike("Facturen").contains(&"invoice".to_string()), "a plural and a capital");
        assert!(alike("Rechnungen").contains(&"factuur".to_string()), "a German plural");
        assert!(alike("réunion").contains(&"vergadering".to_string()), "with an accent");
        assert!(alike("Bestätigung").contains(&"bevestiging".to_string()), "an umlaut");
    }

    #[test]
    fn unknown_short_and_several_words_find_nothing() {
        assert!(alike("keukenkastje").is_empty());
        assert!(alike("de").is_empty());
        assert!(alike("2026").is_empty());
        assert!(alike("nieuwe offerte").is_empty(), "one word at a time");
    }

    #[test]
    fn words_that_only_look_alike_stay_apart() {
        let finds = |word: &str, other: &str| alike(word).iter().any(|w| w == other);
        assert!(!finds("auto", "salary"), "Wagen is not wages");
        assert!(!finds("vakantie", "vacature"), "vacances is not vacancy");
        assert!(!finds("training", "trein"), "a training is no train");
        assert!(!finds("btw", "vader"), "VAT is not Vater");
        assert!(!finds("wifi", "echtgenote"), "wifi is not a wife");
        assert!(!finds("voorstel", "theatre"), "a proposal is no performance");
        assert!(!finds("storing", "winkel"), "a malfunction is no store");
        assert!(finds("train", "trein"), "a word marked as written is still found as written");
        assert!(finds("facturen", "invoice") && finds("betaling", "zahlung"), "the stems that belong together still do");
    }

    #[test]
    fn the_list_knows_daily_life_too() {
        let finds = |word: &str, other: &str| alike(word).iter().any(|w| w == other);
        assert!(finds("maandag", "lundi") && finds("Montag", "monday"));
        assert!(finds("huisarts", "doctor") && finds("Zahnarzt", "tandarts"));
        assert!(finds("verhuizing", "umzug") && finds("loodgieter", "plombier"));
        assert!(finds("rijbewijs", "führerschein") && finds("fuhrerschein", "rijbewijs"), "with or without the umlaut");
        assert!(finds("Keulen", "köln") && finds("kinderopvang", "childcare"));
    }

    #[test]
    fn every_line_of_the_list_is_read() {
        let lines = LIST.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).count();
        assert!(lines > 900, "{lines}");
        assert!(entries().iter().all(|e| !e.word.starts_with('=')), "the mark is not part of the word");
        assert!(entries().iter().any(|e| e.exact));
        assert!(entries().iter().all(|e| !e.word.contains(' ')), "single words only");
        assert_eq!(entries().iter().map(|e| e.line).max().map(|l| l + 1), Some(lines));
    }
}
