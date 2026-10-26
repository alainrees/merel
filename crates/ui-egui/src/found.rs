//! The words a search found, marked in the message being read, as Outlook marks them: a yellow
//! ground behind each word, in any form the index found it (another ending, without accents,
//! with the typing error the search box had).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::Ui;
use egui::text::{LayoutJob, TextFormat};
use merel_engine::Session;
use merel_engine::search::Query;
use merel_html::{Block, Document, Inline};

use crate::Tokens;

/// Per piece of text, the byte ranges of the words found in it.
pub type Marks = HashMap<String, Vec<Range<usize>>>;

/// The words of the search box to mark: the plain words, those of `OF` groups and of phrases.
/// Names, subjects and amounts were searched in their own field and are left alone.
pub fn words(q: &Query) -> Vec<String> {
    q.words.iter().chain(q.either.iter().flatten()).chain(&q.phrases).cloned().collect()
}

/// The marks for `texts`, the pieces of the message `key`, for the search as it stands; worked
/// out once per message and search and kept in egui's memory. `None` when nothing is searched
/// or nothing was found.
pub fn marks(ui: &Ui, key: &str, session: &Session, texts: &[&str]) -> Option<Arc<Marks>> {
    let q = session.search_query();
    let words = words(&q);
    if words.is_empty() {
        return None;
    }
    // Switched to only what was typed: another answer.
    let search = format!("{}\u{1}{}", session.search, session.search_exact);
    let search = search.as_str();
    // One answer kept per message, for the search it was for.
    let id = egui::Id::new(("found", key));
    if let Some((for_search, cached)) = ui.data(|d| d.get_temp::<(String, Option<Arc<Marks>>)>(id))
        && for_search == search
    {
        return cached;
    }
    let mut marker = merel_search::Marker::new(words.iter().map(String::as_str)).also(q.alike.iter().flatten().map(String::as_str));
    let found: Marks = texts.iter().zip(marker.marks(texts)).filter(|(_, m)| !m.is_empty()).map(|(t, m)| (t.to_string(), m)).collect();
    let found = (!found.is_empty()).then(|| Arc::new(found));
    ui.data_mut(|d| d.insert_temp(id, (search.to_string(), found.clone())));
    found
}

/// Every piece of text an HTML message draws, so [`marks`] can look at them all at once.
pub fn texts_of(doc: &Document) -> Vec<&str> {
    fn blocks<'a>(list: &'a [Block], out: &mut Vec<&'a str>) {
        for b in list {
            match b {
                Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                    for i in inlines {
                        if let Inline::Text { text, .. } | Inline::Link { text, .. } = i {
                            out.push(text);
                        }
                    }
                }
                Block::Quote(inner) => blocks(inner, out),
                Block::List { items, .. } => items.iter().for_each(|item| blocks(item, out)),
                Block::Table(rows) => rows.iter().flatten().for_each(|cell| blocks(&cell.blocks, out)),
                Block::Code(_) | Block::Rule => {}
            }
        }
    }
    let mut out = Vec::new();
    blocks(&doc.blocks, &mut out);
    out
}

/// `text` cut at `marks` into pieces, each with whether it was found.
pub fn pieces<'a>(text: &'a str, marks: &[Range<usize>]) -> Vec<(&'a str, bool)> {
    let mut out = Vec::new();
    let mut at = 0;
    for r in marks {
        if r.start < at || r.end > text.len() || !text.is_char_boundary(r.start) || !text.is_char_boundary(r.end) {
            continue;
        }
        if r.start > at {
            out.push((&text[at..r.start], false));
        }
        out.push((&text[r.clone()], true));
        at = r.end;
    }
    if at < text.len() {
        out.push((&text[at..], false));
    }
    out
}

/// The format of a word found: the text's own, on the yellow ground.
pub fn marked(base: &TextFormat, t: &Tokens) -> TextFormat {
    TextFormat { color: t.on_found, background: t.found, ..base.clone() }
}

/// `text` as one layout with the words found on the yellow ground.
pub fn job(text: &str, marks: &[Range<usize>], base: TextFormat, t: &Tokens) -> LayoutJob {
    let mut job = LayoutJob::default();
    let found = marked(&base, t);
    for (piece, hit) in pieces(text, marks) {
        job.append(piece, 0.0, if hit { found.clone() } else { base.clone() });
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_to_mark_are_the_plain_ones_either_and_phrases() {
        let w = |s: &str| words(&Query::parse(s));
        assert_eq!(w("van:sanne factuur \"nieuwe keuken\" a OF b bedrag:>100 -oud"), ["factuur", "a", "b", "nieuwe keuken"]);
        assert!(w("van:sanne is:ongelezen groot:>5MB").is_empty(), "nothing to mark in the text");
    }

    #[test]
    fn a_text_is_cut_around_what_was_found() {
        let text = "De factuur en nog een factuur.";
        assert_eq!(pieces(text, &[3..10, 22..29]), [("De ", false), ("factuur", true), (" en nog een ", false), ("factuur", true), (".", false)]);
        assert_eq!(pieces(text, &[]), [(text, false)]);
        assert_eq!(pieces("é", &[0..1, 3..9]), [("é", false)], "never inside a character, nor past the end");
    }
}
