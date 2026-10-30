//! Search on meaning, next to the search index ([`crate::indexing`]): the model on its own
//! thread ([`merel_semantic::Worker`]), a vector for every message the index holds the text
//! of, and the search box's words as a vector too. Messages whose vectors lie close to the
//! search's join the index's hits, mixed in by rank (reciprocal rank fusion), so a message
//! that has the words stays on top and one that is only about the same thing comes along.
//!
//! The model also learns the person's own words ([`Meaning::learned`]): it reads the words
//! their mail uses most, one by one, and a search word whose vector lies closest to another
//! word's, both ways, has that word searched along, as the built-in list of words in four
//! languages has ([`merel_search::lexicon`]). So mail that says "Kostenvoranschlag" or
//! "devis" is found with "offerte", also where the list does not know the word.
//!
//! Off unless the person switches it on in "Bestand > Zoeken"; switching it off throws away
//! the model and every vector. The vectors say what mail is about, so they are sealed on disk
//! with the index's key and live in memory only.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use merel_search::{Index, Sealer};
use merel_semantic::{Reply, Request, Vectors, Worker};

/// Name of the sealed file with the vectors, in the index's directory.
pub const VECTORS_FILE: &str = "meaning.vec";
/// Name of the sealed file with the vectors of the mail's words.
pub const WORDS_FILE: &str = "words.vec";
/// Ids of the words at the model, apart from the messages': a word of the mail, and a search
/// word the mail does not use often.
const WORD: &str = "\u{1}word:";
const SEARCH_WORD: &str = "\u{1}search:";
/// How many of the mail's words the model reads, the most used first; each in at least this
/// many messages and in no more than one in this many, which leaves out "de" and "the".
const VOCABULARY: usize = 20_000;
const WORD_MIN_DOCS: u32 = 3;
const WORD_MAX_SHARE: u32 = 5;
/// How often the mail's words are looked at again.
const RESCAN: Duration = Duration::from_secs(6 * 3600);
/// Below this many words there is too little to learn from.
const LEAST_WORDS: usize = 200;
/// A word is learned for a search word when it is at least this close, not much less than the
/// closest, the search word is among its own closest few, and it is not the same word in
/// another form; at most a few per search word.
const WORD_SIMILARITY: f32 = 0.88;
const WORD_BELOW_BEST: f32 = 0.04;
const WORD_NEIGHBOURS: usize = 12;
const MUTUAL: usize = 3;
const MOST_LEARNED: usize = 3;
/// How many messages wait at the model at a time.
const IN_FLIGHT: usize = 4;
/// How many ids one look at the index takes, catching up on messages without a vector.
const PAGE: usize = 500;
/// How much of a message the model reads: the subject and the start of the text.
const TEXT_CHARS: usize = 1500;
/// How often changed vectors are written.
const SAVE_EVERY: Duration = Duration::from_secs(60);
/// The most messages a search adds on meaning alone.
const MAX_RELATED: usize = 30;
/// How close a message must be to count as about the same thing: at least this similar, and
/// not much less than the closest one.
const MIN_SIMILARITY: f32 = 0.80;
const BELOW_BEST: f32 = 0.08;
/// The k of reciprocal rank fusion.
const FUSION_K: f32 = 60.0;

/// How many megabytes switching it on downloads.
pub fn download_megabytes() -> u64 {
    merel_semantic::setup::download_megabytes()
}

/// Where getting the model is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ModelState {
    #[default]
    Off,
    Downloading {
        done: u64,
        total: u64,
    },
    Preparing,
    Ready,
    Failed(String),
}

/// What "Bestand > Zoeken" shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeaningStatus {
    pub model: ModelState,
    /// Messages with a vector.
    pub vectors: usize,
    /// Words of the mail with a vector, to learn from ([`Meaning::learned`]).
    pub words: usize,
    /// Whether messages are still being read.
    pub busy: bool,
    /// Bytes the prepared model takes.
    pub model_size: u64,
}

pub struct Meaning {
    model_dir: PathBuf,
    /// Where the vectors are sealed; `None` in memory only (tests).
    index_dir: Option<PathBuf>,
    sealer: Option<Arc<dyn Sealer>>,
    worker: Worker,
    state: ModelState,
    vectors: Vectors,
    dirty: bool,
    saved: Instant,
    /// Messages at the model now.
    pending: HashSet<String>,
    /// Messages known to need a vector, newest first.
    todo: Vec<String>,
    /// How far the look through the index for messages without a vector got; `None` once done.
    backfill: Option<usize>,
    /// The search's words as the model read them, and the words asked about last.
    query: Option<(String, Vec<f32>)>,
    asked: Option<String>,
    /// How close counts as about the same thing; see [`MIN_SIMILARITY`].
    min_similarity: f32,
    below_best: f32,
    /// The vectors of the mail's words, and whether they changed since they were written.
    words: Vectors,
    words_dirty: bool,
    /// Words still to be read, and when the mail's words were last looked at.
    word_todo: Vec<String>,
    scanned: Option<Instant>,
    /// Vectors of search words the mail does not use often, and the words asked about.
    search_words: HashMap<String, Vec<f32>>,
    asked_words: HashSet<String>,
    /// The words learned for a search word, once all words are read.
    learned: HashMap<String, Vec<String>>,
    /// How close counts as the same word; see [`WORD_SIMILARITY`].
    word_similarity: f32,
}

impl Meaning {
    /// Starts the model's thread; downloads the model when it is not there yet.
    pub fn start(model_dir: PathBuf, index_dir: Option<PathBuf>, sealer: Option<Arc<dyn Sealer>>, slow: bool) -> Meaning {
        let worker = Worker::start(model_dir.clone(), slow);
        let ready = merel_semantic::setup::is_ready(&model_dir);
        if !ready {
            worker.send(Request::Install);
        }
        let read = |file: &str| match (&index_dir, &sealer) {
            (Some(dir), Some(sealer)) => load(dir, sealer.as_ref(), file),
            _ => None,
        };
        let vectors = read(VECTORS_FILE).unwrap_or_else(|| Vectors::new(merel_semantic::DIM));
        let words = read(WORDS_FILE).unwrap_or_else(|| Vectors::new(merel_semantic::DIM));
        Meaning {
            model_dir,
            index_dir,
            sealer,
            worker,
            state: if ready { ModelState::Ready } else { ModelState::Downloading { done: 0, total: 0 } },
            vectors,
            dirty: false,
            saved: Instant::now(),
            pending: HashSet::new(),
            todo: Vec::new(),
            backfill: Some(0),
            query: None,
            asked: None,
            min_similarity: MIN_SIMILARITY,
            below_best: BELOW_BEST,
            words,
            words_dirty: false,
            word_todo: Vec::new(),
            scanned: None,
            search_words: HashMap::new(),
            asked_words: HashSet::new(),
            learned: HashMap::new(),
            word_similarity: WORD_SIMILARITY,
        }
    }

    pub fn status(&self) -> MeaningStatus {
        MeaningStatus {
            model: self.state.clone(),
            vectors: self.vectors.len(),
            words: self.words.len(),
            busy: !self.pending.is_empty() || !self.todo.is_empty() || self.backfill.is_some() || !self.word_todo.is_empty(),
            model_size: merel_semantic::setup::size_on_disk(&self.model_dir),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.state == ModelState::Ready
    }

    /// Tries the download again after it failed.
    pub fn retry(&mut self) {
        if matches!(self.state, ModelState::Failed(_)) {
            self.state = ModelState::Downloading { done: 0, total: 0 };
            self.worker.send(Request::Install);
        }
    }

    /// A message whose text just went into the index.
    pub fn note_text(&mut self, id: &str) {
        self.dirty |= self.vectors.remove(id);
        self.todo.retain(|t| t != id);
        self.todo.insert(0, id.to_string());
    }

    pub fn forget(&mut self, id: &str) {
        self.dirty |= self.vectors.remove(id);
        self.todo.retain(|t| t != id);
    }

    pub fn forget_where(&mut self, gone: impl Fn(&str) -> bool) {
        let before = self.vectors.len();
        self.vectors.retain(|id| !gone(id));
        self.dirty |= self.vectors.len() != before;
        self.todo.retain(|id| !gone(id));
    }

    /// Takes in what the model answered, hands it more messages, and writes the vectors when
    /// it is time. True when a search's words came back, so the search is due again.
    pub fn tick(&mut self, index: &Index, force_save: bool) -> bool {
        let mut query_came = false;
        while let Some(reply) = self.worker.try_recv() {
            match reply {
                Reply::Progress(merel_semantic::Stage::Downloading { done, total }) => self.state = ModelState::Downloading { done, total },
                Reply::Progress(merel_semantic::Stage::Preparing) => self.state = ModelState::Preparing,
                Reply::Installed => self.state = ModelState::Ready,
                Reply::Failed(e) => {
                    if self.state == ModelState::Ready {
                        log::info!("search on meaning: {e}");
                        self.pending.clear();
                    } else {
                        log::warn!("search on meaning: no model: {e}");
                        self.state = ModelState::Failed(e);
                    }
                }
                Reply::Vector { id, vector } => {
                    if !self.pending.remove(&id) {
                        continue;
                    }
                    if let Some(word) = id.strip_prefix(WORD) {
                        if vector.len() != self.words.dim() {
                            self.words = Vectors::new(vector.len());
                            self.scanned = None;
                        }
                        self.words.insert(word, &vector);
                        self.words_dirty = true;
                        self.learned.clear();
                    } else if let Some(word) = id.strip_prefix(SEARCH_WORD) {
                        self.search_words.insert(word.to_string(), vector);
                        query_came = true;
                    } else {
                        if vector.len() != self.vectors.dim() {
                            // Another model than the vectors on disk came from: read all mail again.
                            self.vectors = Vectors::new(vector.len());
                            self.backfill = Some(0);
                        }
                        self.vectors.insert(&id, &vector);
                        self.dirty = true;
                    }
                }
                Reply::QueryVector { text, vector } => {
                    self.query = Some((text, vector));
                    query_came = true;
                }
            }
        }
        if self.is_ready() {
            self.feed(index);
        }
        if (self.dirty || self.words_dirty) && (force_save || self.saved.elapsed() >= SAVE_EVERY) {
            self.save();
        }
        query_came
    }

    fn feed(&mut self, index: &Index) {
        if self.todo.is_empty()
            && let Some(offset) = self.backfill
        {
            let page = index.ids_with_text(offset, PAGE);
            self.backfill = if page.len() < PAGE { None } else { Some(offset + PAGE) };
            self.todo.extend(page.into_iter().filter(|id| !self.vectors.contains(id) && !self.pending.contains(id)));
        }
        while self.pending.len() < IN_FLIGHT && !self.todo.is_empty() {
            let id = self.todo.remove(0);
            if self.vectors.contains(&id) || self.pending.contains(&id) {
                continue;
            }
            let Some((header, text)) = index.stored_text(&id) else { continue };
            let text: String = format!("{}\n{}", header.subject, text).chars().take(TEXT_CHARS).collect();
            if text.trim().is_empty() {
                continue;
            }
            self.pending.insert(id.clone());
            self.worker.send(Request::Embed { id, text });
        }
        // The mail's words once the messages are read, and again now and then.
        if self.todo.is_empty() && self.backfill.is_none() && self.scanned.is_none_or(|t| t.elapsed() >= RESCAN) {
            self.scanned = Some(Instant::now());
            let vocabulary: HashSet<String> = index.common_words(VOCABULARY, WORD_MIN_DOCS, WORD_MAX_SHARE).into_iter().collect();
            let before = self.words.len();
            self.words.retain(|w| vocabulary.contains(w));
            self.words_dirty |= self.words.len() != before;
            self.word_todo = vocabulary.into_iter().filter(|w| !self.words.contains(w)).collect();
            self.word_todo.sort();
        }
        while self.pending.len() < IN_FLIGHT
            && let Some(word) = self.word_todo.pop()
        {
            let id = format!("{WORD}{word}");
            if self.pending.insert(id.clone()) {
                self.worker.send(Request::Embed { id, text: word });
            }
        }
    }

    /// The words of the person's own mail that mean what `word` means, for the search to look
    /// for too; empty when there are none or the model is not there. `None` while the model
    /// still reads the word, which it is asked to here.
    pub fn learned(&mut self, word: &str) -> Option<Vec<String>> {
        let Some(word) = merel_search::lexicon::folded(word) else { return Some(Vec::new()) };
        if !self.is_ready() || self.words.len() < LEAST_WORDS || word.chars().count() < 4 || !word.chars().all(char::is_alphabetic) {
            return Some(Vec::new());
        }
        if let Some(known) = self.learned.get(&word) {
            return Some(known.clone());
        }
        let Some(vector) = self.words.get(&word).or_else(|| self.search_words.get(&word).cloned()) else {
            if self.asked_words.insert(word.clone()) {
                let id = format!("{SEARCH_WORD}{word}");
                self.pending.insert(id.clone());
                self.worker.send(Request::Embed { id, text: word });
            }
            return None;
        };
        let found = neighbours(&self.words, &word, &vector, self.word_similarity);
        // While words are still being read, the answer may change.
        if self.word_todo.is_empty() {
            self.learned.insert(word, found.clone());
        }
        Some(found)
    }

    /// The messages about the same thing as `words`, with how close each is, best first;
    /// `allowed` says which are in the search's scope. Empty until the model read the words,
    /// which it is asked to here.
    pub fn related(&mut self, words: &str, allowed: impl Fn(&str) -> bool) -> Vec<(String, f32)> {
        if !self.is_ready() || words.trim().is_empty() {
            return Vec::new();
        }
        match &self.query {
            Some((text, vector)) if text == words => {
                let near = self.vectors.nearest(vector, MAX_RELATED * 2, allowed);
                let best = near.first().map(|x| x.1).unwrap_or(0.0);
                near.into_iter().filter(|(_, s)| *s >= self.min_similarity && *s >= best - self.below_best).take(MAX_RELATED).collect()
            }
            _ => {
                if self.asked.as_deref() != Some(words) {
                    self.asked = Some(words.to_string());
                    self.worker.send(Request::Query { text: words.to_string() });
                }
                Vec::new()
            }
        }
    }

    /// For tests with a model that knows no meaning: every message counts as close.
    #[cfg(test)]
    pub(crate) fn loosen(&mut self) {
        self.min_similarity = -1.0;
        self.below_best = 2.0;
    }

    fn save(&mut self) {
        let (Some(dir), Some(sealer)) = (&self.index_dir, &self.sealer) else {
            self.dirty = false;
            self.words_dirty = false;
            return;
        };
        let write = |file: &str, vectors: &Vectors| sealer.seal(file, &vectors.to_bytes()).and_then(|sealed| std::fs::write(dir.join(file), sealed));
        if self.dirty {
            match write(VECTORS_FILE, &self.vectors) {
                Ok(()) => self.dirty = false,
                Err(e) => log::warn!("search on meaning: vectors not written: {e}"),
            }
        }
        if self.words_dirty {
            match write(WORDS_FILE, &self.words) {
                Ok(()) => self.words_dirty = false,
                Err(e) => log::warn!("search on meaning: words not written: {e}"),
            }
        }
        self.saved = Instant::now();
    }

    pub fn flush(&mut self) {
        if self.dirty || self.words_dirty {
            self.save();
        }
    }

    /// Forgets every vector, on disk too (the index was emptied).
    pub fn clear(&mut self) {
        self.vectors.clear();
        self.todo.clear();
        self.pending.clear();
        self.backfill = Some(0);
        self.dirty = false;
        self.words.clear();
        self.word_todo.clear();
        self.scanned = None;
        self.search_words.clear();
        self.asked_words.clear();
        self.learned.clear();
        self.words_dirty = false;
        if let Some(dir) = &self.index_dir {
            let _ = std::fs::remove_file(dir.join(VECTORS_FILE));
            let _ = std::fs::remove_file(dir.join(WORDS_FILE));
        }
    }

    /// Switches off: the vectors and the model go.
    pub fn remove(mut self) -> Result<(), String> {
        self.clear();
        let dir = self.model_dir.clone();
        // The thread lets go of the model first.
        drop(self);
        merel_semantic::setup::remove(&dir).map_err(|e| e.to_string())
    }
}

fn load(dir: &Path, sealer: &dyn Sealer, file: &str) -> Option<Vectors> {
    let bytes = std::fs::read(dir.join(file)).ok()?;
    Vectors::from_bytes(&sealer.open(file, bytes)?)
}

/// The words among `words` that mean what `word` (with vector `v`) means: close to it, at least
/// `floor` and not much less than the closest, with `word` among their own closest few (two
/// words that are each other's nearest are what a translation looks like), and not the same
/// word in another form, which the search finds anyway. The closest first, a few at most.
pub fn neighbours(words: &Vectors, word: &str, v: &[f32], floor: f32) -> Vec<String> {
    let near = words.nearest(v, WORD_NEIGHBOURS, |w| w != word);
    let best = near.first().map(|x| x.1).unwrap_or(0.0);
    let mut out = Vec::new();
    for (other, similarity) in near {
        if similarity < floor || similarity < best - WORD_BELOW_BEST || out.len() == MOST_LEARNED {
            break;
        }
        if merel_search::lexicon::same_word(word, &other) || out.iter().any(|w: &String| merel_search::lexicon::same_word(w, &other)) {
            continue;
        }
        let Some(theirs) = words.get(&other) else { continue };
        // How close `word` is to the other word, against the other word's own closest.
        let back: f32 = theirs.iter().zip(v).map(|(a, b)| a * b).sum();
        let closest = words.nearest(&theirs, MUTUAL, |w| w != other && w != word);
        if closest.len() < MUTUAL || closest.last().is_some_and(|(_, s)| back >= *s) {
            out.push(other);
        }
    }
    out
}

/// Mixes the index's hits (best first) with the related messages (best first) by rank: each
/// message scores 1/(k + rank) in each list it is in.
pub fn fuse(hits: &[String], related: &[String]) -> Vec<(String, f32)> {
    let mut scores: Vec<(String, f32)> = Vec::new();
    for (list, weight) in [(hits, 1.0), (related, 1.0)] {
        for (rank, id) in list.iter().enumerate() {
            let s = weight / (FUSION_K + rank as f32 + 1.0);
            match scores.iter_mut().find(|(x, _)| x == id) {
                Some(entry) => entry.1 += s,
                None => scores.push((id.clone(), s)),
            }
        }
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    #[test]
    fn a_word_is_learned_when_both_words_are_each_others_closest() {
        let mut words = Vectors::new(6);
        let put = |words: &mut Vectors, w: &str, v: &[f32]| words.insert(w, &unit(v));
        put(&mut words, "offerte", &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        put(&mut words, "angebot", &[1.0, 0.15, 0.0, 0.0, 0.0, 0.0]);
        put(&mut words, "offertes", &[1.0, 0.0, 0.1, 0.0, 0.0, 0.0]);
        // Close to "offerte", but closer still to three words of its own: no translation of it.
        put(&mut words, "document", &[1.0, 0.0, 0.0, 0.6, 0.0, 0.0]);
        for (w, x) in [("bestand", 0.62), ("bijlage", 0.64), ("pagina", 0.66)] {
            put(&mut words, w, &[1.0, 0.0, 0.0, x, 0.05, 0.0]);
        }
        put(&mut words, "keuken", &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        let v = words.get("offerte").unwrap();
        assert_eq!(neighbours(&words, "offerte", &v, 0.80), ["angebot"]);
        assert!(neighbours(&words, "offerte", &v, 0.999).is_empty(), "not close enough");
        let k = words.get("keuken").unwrap();
        assert!(neighbours(&words, "keuken", &k, 0.80).is_empty(), "nothing near");
        // A search word the mail does not use, with a vector of its own.
        let quote = unit(&[1.0, 0.12, 0.0, 0.0, 0.0, 0.0]);
        assert!(neighbours(&words, "quote", &quote, 0.80).contains(&"angebot".to_string()));
    }

    #[test]
    fn fusion_keeps_what_both_lists_found_on_top_and_adds_the_rest() {
        let hits = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let related = vec!["c".to_string(), "x".to_string()];
        let fused: Vec<String> = fuse(&hits, &related).into_iter().map(|(id, _)| id).collect();
        assert_eq!(fused, ["c", "a", "b", "x"]);
        assert_eq!(fuse(&hits, &[]).into_iter().map(|(id, _)| id).collect::<Vec<_>>(), ["a", "b", "c"], "nothing related: as the index ranked");
    }
}
