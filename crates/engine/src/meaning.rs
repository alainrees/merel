//! Search on meaning, next to the search index ([`crate::indexing`]): the model on its own
//! thread ([`merel_semantic::Worker`]), a vector for every message the index holds the text
//! of, and the search box's words as a vector too. Messages whose vectors lie close to the
//! search's join the index's hits, mixed in by rank (reciprocal rank fusion), so a message
//! that has the words stays on top and one that is only about the same thing comes along.
//!
//! Off unless the person switches it on in "Bestand > Zoeken"; switching it off throws away
//! the model and every vector. The vectors say what mail is about, so they are sealed on disk
//! with the index's key and live in memory only.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use merel_search::{Index, Sealer};
use merel_semantic::{Reply, Request, Vectors, Worker};

/// Name of the sealed file with the vectors, in the index's directory.
pub const VECTORS_FILE: &str = "meaning.vec";
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
}

impl Meaning {
    /// Starts the model's thread; downloads the model when it is not there yet.
    pub fn start(model_dir: PathBuf, index_dir: Option<PathBuf>, sealer: Option<Arc<dyn Sealer>>, slow: bool) -> Meaning {
        let worker = Worker::start(model_dir.clone(), slow);
        let ready = merel_semantic::setup::is_ready(&model_dir);
        if !ready {
            worker.send(Request::Install);
        }
        let vectors = match (&index_dir, &sealer) {
            (Some(dir), Some(sealer)) => load(dir, sealer.as_ref()),
            _ => None,
        }
        .unwrap_or_else(|| Vectors::new(merel_semantic::DIM));
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
        }
    }

    pub fn status(&self) -> MeaningStatus {
        MeaningStatus {
            model: self.state.clone(),
            vectors: self.vectors.len(),
            busy: !self.pending.is_empty() || !self.todo.is_empty() || self.backfill.is_some(),
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
                    if self.pending.remove(&id) {
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
        if self.dirty && (force_save || self.saved.elapsed() >= SAVE_EVERY) {
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
            return;
        };
        match sealer.seal(VECTORS_FILE, &self.vectors.to_bytes()).and_then(|sealed| std::fs::write(dir.join(VECTORS_FILE), sealed)) {
            Ok(()) => {
                self.dirty = false;
                self.saved = Instant::now();
            }
            Err(e) => log::warn!("search on meaning: vectors not written: {e}"),
        }
    }

    pub fn flush(&mut self) {
        if self.dirty {
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
        if let Some(dir) = &self.index_dir {
            let _ = std::fs::remove_file(dir.join(VECTORS_FILE));
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

fn load(dir: &Path, sealer: &dyn Sealer) -> Option<Vectors> {
    let bytes = std::fs::read(dir.join(VECTORS_FILE)).ok()?;
    Vectors::from_bytes(&sealer.open(VECTORS_FILE, bytes)?)
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

    #[test]
    fn fusion_keeps_what_both_lists_found_on_top_and_adds_the_rest() {
        let hits = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let related = vec!["c".to_string(), "x".to_string()];
        let fused: Vec<String> = fuse(&hits, &related).into_iter().map(|(id, _)| id).collect();
        assert_eq!(fused, ["c", "a", "b", "x"]);
        assert_eq!(fuse(&hits, &[]).into_iter().map(|(id, _)| id).collect::<Vec<_>>(), ["a", "b", "c"], "nothing related: as the index ranked");
    }
}
