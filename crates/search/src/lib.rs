//! The search index: the words of every message on this computer, as Outlook and Apple Mail
//! keep one, so a search covers all mail of all accounts at once, old mail included, works
//! without a connection and answers in milliseconds.
//!
//! Built on Tantivy. Each message is one document: its header (kept whole, so a hit can be
//! listed without the server), the sender, the recipients, the subject, the text and the names
//! of the attachments. Words are lower-cased and stripped of accents, so `cafe` finds `café`;
//! they are also kept as Dutch and English stems, so `factuur` finds `facturen`. A word that
//! is not there at all is still found with a typing error or two (`fatcuur`), and the word
//! being typed counts as the start of a word. Hits are ranked by how well they match, with a
//! push for recent mail and for people the person knows.
//!
//! The index is sealed on disk and lives decrypted in memory only; see [`sealed`].

pub mod sealed;

use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use merel_model::MessageHeader;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, RangeQuery, TermQuery};
use tantivy::schema::{FAST, Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value};
use tantivy::tokenizer::{AsciiFoldingFilter, Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer};
use tantivy::{DocAddress, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

pub use sealed::{SealedDirectory, Sealer};

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("search index: {0}")]
    Index(#[from] tantivy::TantivyError),
    #[error("search index: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SearchError>;

/// How much memory the writer may fill before it writes a segment.
const WRITER_MEMORY: usize = 20_000_000;
/// How long changes wait before they are written and searchable.
const COMMIT_AFTER: Duration = Duration::from_secs(5);
/// The most text of one message that is indexed and kept for the snippet.
pub const MAX_TEXT: usize = 64 * 1024;
/// How much of a hit's text is read for its snippet.
const SNIPPET_SCAN: usize = 16 * 1024;
/// How long a snippet is, about.
const SNIPPET_CHARS: usize = 160;

const FOLD: &str = "merel_fold";
const STEM_NL: &str = "merel_nl";
const STEM_EN: &str = "merel_en";

#[derive(Clone, Copy)]
struct Fields {
    id: Field,
    account: Field,
    folder: Field,
    header: Field,
    full: Field,
    date: Field,
    subject: Field,
    from: Field,
    to: Field,
    text: Field,
    attachments: Field,
    stem_nl: Field,
    stem_en: Field,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let words = |name: &str, stored: bool| {
        let o = TextOptions::default()
            .set_indexing_options(TextFieldIndexing::default().set_tokenizer(name).set_index_option(IndexRecordOption::WithFreqsAndPositions));
        if stored { o.set_stored() } else { o }
    };
    let stems = |name: &str| {
        TextOptions::default().set_indexing_options(TextFieldIndexing::default().set_tokenizer(name).set_index_option(IndexRecordOption::WithFreqs))
    };
    let fields = Fields {
        id: b.add_text_field("id", STRING | STORED),
        account: b.add_text_field("account", STRING),
        folder: b.add_text_field("folder", STRING | STORED),
        header: b.add_text_field("header", STORED),
        full: b.add_u64_field("full", INDEXED | STORED),
        date: b.add_i64_field("date", INDEXED | FAST | STORED),
        subject: b.add_text_field("subject", words(FOLD, false)),
        from: b.add_text_field("from", words(FOLD, false)),
        to: b.add_text_field("to", words(FOLD, false)),
        text: b.add_text_field("text", words(FOLD, true)),
        attachments: b.add_text_field("attachments", words(FOLD, false)),
        stem_nl: b.add_text_field("stem_nl", stems(STEM_NL)),
        stem_en: b.add_text_field("stem_en", stems(STEM_EN)),
    };
    (b.build(), fields)
}

fn fold() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default()).filter(RemoveLongFilter::limit(40)).filter(LowerCaser).filter(AsciiFoldingFilter).build()
}

fn stemmed(language: Language) -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .filter(Stemmer::new(language))
        .build()
}

/// The words of `text` as the index keeps them, with where each stands in `text`.
fn tokens(analyzer: &mut TextAnalyzer, text: &str) -> Vec<(String, Range<usize>)> {
    let mut out = Vec::new();
    let mut stream = analyzer.token_stream(text);
    while stream.advance() {
        let t = stream.token();
        out.push((t.text.clone(), t.offset_from..t.offset_to));
    }
    out
}

/// What to look for: the words of the search box, as [`merel_engine::search::Query`] read them.
#[derive(Clone, Copy, Debug, Default)]
pub struct Terms<'a> {
    /// Words that may occur anywhere.
    pub words: &'a [String],
    pub from: &'a [String],
    pub to: &'a [String],
    pub subject: &'a [String],
}

impl Terms<'_> {
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.from.is_empty() && self.to.is_empty() && self.subject.is_empty()
    }
}

/// Where and how to search.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    /// Only these folders; `None` for all mail.
    pub folders: Option<Vec<String>>,
    /// The most hits returned.
    pub limit: usize,
    /// Now, in seconds since the epoch, for the push recent mail gets.
    pub now: i64,
    /// Addresses (lower case) of people the person knows; their mail ranks a little higher.
    pub known: HashSet<String>,
}

/// A piece of a hit's text around the words found, with where those words are in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// Byte ranges in `text` of the words that matched.
    pub marks: Vec<Range<usize>>,
}

/// One message the index found.
#[derive(Clone, Debug)]
pub struct Hit {
    pub header: MessageHeader,
    pub score: f32,
    pub snippet: Option<Snippet>,
}

/// How far the index is: messages known, and how many of them with their whole text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub messages: u64,
    pub with_text: u64,
}

pub struct Index {
    writer: IndexWriter,
    reader: IndexReader,
    fields: Fields,
    directory: SealedDirectory,
    fold: TextAnalyzer,
    stem_nl: TextAnalyzer,
    stem_en: TextAnalyzer,
    /// Ids written or removed since the last commit, which the reader does not see yet.
    added: HashSet<String>,
    removed: HashSet<String>,
    dirty: bool,
    last_commit: Instant,
    counts: Counts,
    /// Bumped on every commit, so a search can tell that its answer may have changed.
    generation: u64,
}

impl Index {
    /// An index that lives in memory only.
    pub fn in_memory() -> Result<Index> {
        Index::with_directory(SealedDirectory::in_memory())
    }

    /// The index sealed in `dir`, read into memory; a fresh one when there is none yet.
    pub fn open(dir: &Path, sealer: Arc<dyn Sealer>) -> Result<Index> {
        let directory = SealedDirectory::open(dir, Arc::clone(&sealer))?;
        match Index::with_directory(directory) {
            Ok(index) => Ok(index),
            Err(e) => {
                // An index from an older layout, or a broken one: start again.
                log::warn!("{}: search index not opened ({e}), building it again", dir.display());
                sealed::wipe(dir)?;
                Index::with_directory(SealedDirectory::open(dir, sealer)?)
            }
        }
    }

    fn with_directory(directory: SealedDirectory) -> Result<Index> {
        let (schema, fields) = schema();
        let index = tantivy::IndexBuilder::new().schema(schema).open_or_create(directory.clone())?;
        index.tokenizers().register(FOLD, fold());
        index.tokenizers().register(STEM_NL, stemmed(Language::Dutch));
        index.tokenizers().register(STEM_EN, stemmed(Language::English));
        let writer = index.writer_with_num_threads(1, WRITER_MEMORY)?;
        let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;
        let mut me = Index {
            writer,
            reader,
            fields,
            directory,
            fold: fold(),
            stem_nl: stemmed(Language::Dutch),
            stem_en: stemmed(Language::English),
            added: HashSet::new(),
            removed: HashSet::new(),
            dirty: false,
            last_commit: Instant::now(),
            counts: Counts::default(),
            generation: 0,
        };
        me.recount();
        Ok(me)
    }

    /// Bytes the index takes on disk.
    pub fn size(&self) -> u64 {
        self.directory.size()
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Changes with every commit.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn recount(&mut self) {
        let searcher = self.reader.searcher();
        let messages = searcher.search(&AllQuery, &Count).unwrap_or(0) as u64;
        let with_text =
            searcher.search(&TermQuery::new(Term::from_field_u64(self.fields.full, 1), IndexRecordOption::Basic), &Count).unwrap_or(0) as u64;
        self.counts = Counts { messages, with_text };
    }

    fn id_term(&self, id: &str) -> Term {
        Term::from_field_text(self.fields.id, id)
    }

    fn committed_doc(&self, id: &str) -> Option<TantivyDocument> {
        let searcher = self.reader.searcher();
        let query = TermQuery::new(self.id_term(id), IndexRecordOption::Basic);
        let top = searcher.search(&query, &TopDocs::with_limit(1).order_by_score()).ok()?;
        let (_, address) = top.first()?;
        searcher.doc(*address).ok()
    }

    /// Whether the index holds the message, written or waiting to be.
    pub fn contains(&self, id: &str) -> bool {
        self.added.contains(id) || (!self.removed.contains(id) && self.committed_doc(id).is_some())
    }

    /// Whether the index holds the message's whole text (committed).
    pub fn has_text(&self, id: &str) -> bool {
        !self.removed.contains(id) && self.committed_doc(id).and_then(|d| d.get_first(self.fields.full).and_then(|v| v.as_u64())) == Some(1)
    }

    /// The header and account the index keeps for a message.
    pub fn stored(&self, id: &str) -> Option<MessageHeader> {
        if self.removed.contains(id) {
            return None;
        }
        let doc = self.committed_doc(id)?;
        self.header_of(&doc)
    }

    fn header_of(&self, doc: &TantivyDocument) -> Option<MessageHeader> {
        serde_json::from_str(doc.get_first(self.fields.header)?.as_str()?).ok()
    }

    /// Adds a listed message without its text, unless the index has it already.
    pub fn add_header(&mut self, account_id: &str, header: &MessageHeader) -> Result<bool> {
        if self.contains(&header.id) {
            return Ok(false);
        }
        self.write(account_id, header, None, &[])?;
        Ok(true)
    }

    /// Adds or replaces a message with its text and the names of its attachments. `text` is
    /// empty for a message whose content stays out of the index (it came encrypted).
    pub fn add_text(&mut self, account_id: &str, header: &MessageHeader, text: &str, attachments: &[String]) -> Result<()> {
        self.write(account_id, header, Some(text), attachments)
    }

    fn write(&mut self, account_id: &str, header: &MessageHeader, text: Option<&str>, attachments: &[String]) -> Result<()> {
        let f = self.fields;
        self.writer.delete_term(self.id_term(&header.id));
        let mut doc = TantivyDocument::default();
        doc.add_text(f.id, &header.id);
        doc.add_text(f.account, account_id);
        doc.add_text(f.folder, &header.folder_id);
        // The preview is rebuilt from the text when the message opens; the index keeps it small.
        let mut kept = header.clone();
        kept.preview.clear();
        doc.add_text(f.header, serde_json::to_string(&kept).unwrap_or_default());
        doc.add_u64(f.full, u64::from(text.is_some()));
        doc.add_i64(f.date, header.date);
        doc.add_text(f.subject, &header.subject);
        doc.add_text(f.from, format!("{} {}", header.from.display(), header.from.email()));
        let to: Vec<String> = header.to.iter().map(|a| format!("{} {}", a.display(), a.email())).collect();
        doc.add_text(f.to, to.join(" "));
        let text = text.map(|t| cut(t, MAX_TEXT)).unwrap_or_default();
        let names = attachments.join(" ");
        doc.add_text(f.text, text);
        doc.add_text(f.attachments, &names);
        let all = format!("{}\n{}\n{}", header.subject, text, names);
        doc.add_text(f.stem_nl, &all);
        doc.add_text(f.stem_en, &all);
        self.writer.add_document(doc)?;
        self.removed.remove(&header.id);
        self.added.insert(header.id.clone());
        self.dirty = true;
        Ok(())
    }

    pub fn delete(&mut self, id: &str) {
        self.writer.delete_term(self.id_term(id));
        self.added.remove(id);
        self.removed.insert(id.to_string());
        self.dirty = true;
    }

    /// Removes every message of a folder (it went away on the server).
    pub fn delete_folder(&mut self, folder_id: &str) {
        self.writer.delete_term(Term::from_field_text(self.fields.folder, folder_id));
        self.added.retain(|id| !id.starts_with(&format!("{folder_id}#")));
        self.dirty = true;
    }

    /// Removes every message of an account (it was removed from Merel).
    pub fn delete_account(&mut self, account_id: &str) {
        self.writer.delete_term(Term::from_field_text(self.fields.account, account_id));
        self.added.clear();
        self.dirty = true;
    }

    /// Empties the index.
    pub fn clear(&mut self) -> Result<()> {
        self.writer.delete_all_documents()?;
        self.added.clear();
        self.removed.clear();
        self.dirty = true;
        self.commit()
    }

    /// The ids the index holds for a folder from `since` on (committed).
    pub fn ids_in_folder(&self, folder_id: &str, since: i64) -> Vec<String> {
        let f = self.fields;
        let query = BooleanQuery::new(vec![
            (Occur::Must, Box::new(TermQuery::new(Term::from_field_text(f.folder, folder_id), IndexRecordOption::Basic)) as Box<dyn Query>),
            (Occur::Must, Box::new(RangeQuery::new(std::ops::Bound::Included(Term::from_field_i64(f.date, since)), std::ops::Bound::Unbounded))),
        ]);
        self.ids_for(&query, 1_000_000)
    }

    fn ids_for(&self, query: &dyn Query, limit: usize) -> Vec<String> {
        let searcher = self.reader.searcher();
        let Ok(top) = searcher.search(query, &TopDocs::with_limit(limit.max(1)).order_by_score()) else { return Vec::new() };
        top.into_iter()
            .filter_map(|(_, a)| searcher.doc::<TantivyDocument>(a).ok())
            .filter_map(|d| d.get_first(self.fields.id).and_then(|v| v.as_str()).map(str::to_string))
            .filter(|id| !self.removed.contains(id))
            .collect()
    }

    /// The newest messages of an account (from `since` on) whose text the index does not have
    /// yet, at most `n`, newest first.
    pub fn without_text(&self, account_id: &str, since: i64, n: usize) -> Vec<MessageHeader> {
        let f = self.fields;
        let query = BooleanQuery::new(vec![
            (Occur::Must, Box::new(TermQuery::new(Term::from_field_text(f.account, account_id), IndexRecordOption::Basic)) as Box<dyn Query>),
            (Occur::Must, Box::new(TermQuery::new(Term::from_field_u64(f.full, 0), IndexRecordOption::Basic))),
            (Occur::Must, Box::new(RangeQuery::new(std::ops::Bound::Included(Term::from_field_i64(f.date, since)), std::ops::Bound::Unbounded))),
        ]);
        let searcher = self.reader.searcher();
        let collector = TopDocs::with_limit(n.max(1) + self.added.len()).order_by_fast_field::<i64>("date", tantivy::Order::Desc);
        let Ok(top) = searcher.search(&query, &collector) else { return Vec::new() };
        top.into_iter()
            .filter_map(|(_, a)| searcher.doc::<TantivyDocument>(a).ok())
            .filter_map(|d| self.header_of(&d))
            // Written since the last commit: with or without text, it is in hand already.
            .filter(|h| !self.added.contains(&h.id) && !self.removed.contains(&h.id))
            .take(n)
            .collect()
    }

    /// Writes what changed when [`COMMIT_AFTER`] has passed since the last write, or at once
    /// when `force`. Returns whether it wrote.
    pub fn commit_if_due(&mut self, force: bool) -> Result<bool> {
        if !self.dirty || (!force && self.last_commit.elapsed() < COMMIT_AFTER) {
            return Ok(false);
        }
        self.commit()?;
        Ok(true)
    }

    fn commit(&mut self) -> Result<()> {
        self.writer.commit()?;
        self.reader.reload()?;
        self.added.clear();
        self.removed.clear();
        self.dirty = false;
        self.last_commit = Instant::now();
        self.generation += 1;
        self.recount();
        Ok(())
    }

    /// The analysed forms of one search word: folded, and its two stems.
    fn forms(&mut self, word: &str) -> Vec<Form> {
        let folded = tokens(&mut self.fold, word);
        folded
            .into_iter()
            .map(|(t, _)| {
                let nl = tokens(&mut self.stem_nl, &t).into_iter().next().map(|x| x.0).unwrap_or_else(|| t.clone());
                let en = tokens(&mut self.stem_en, &t).into_iter().next().map(|x| x.0).unwrap_or_else(|| t.clone());
                Form { folded: t, nl, en }
            })
            .collect()
    }

    /// Searches the index. `typing` says the last word may still be half typed, so it also
    /// counts as the start of a word.
    pub fn search(&mut self, terms: Terms<'_>, scope: &Scope, typing: bool) -> Vec<Hit> {
        if terms.is_empty() {
            return Vec::new();
        }
        let f = self.fields;
        let mut must: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        let mut all_forms: Vec<(Form, bool)> = Vec::new();
        let last = terms.words.len().saturating_sub(1);
        for (i, word) in terms.words.iter().enumerate() {
            let forms = self.forms(word);
            let n = forms.len();
            for (j, form) in forms.into_iter().enumerate() {
                let prefix = typing && i == last && j + 1 == n;
                must.push((Occur::Must, Box::new(word_query(f, &form, prefix))));
                all_forms.push((form, prefix));
            }
        }
        for (field, words) in [(f.from, terms.from), (f.to, terms.to), (f.subject, terms.subject)] {
            for word in words {
                for form in self.forms(word) {
                    let term = Term::from_field_text(field, &form.folded);
                    must.push((Occur::Must, Box::new(FuzzyTermQuery::new_prefix(term, 0, true))));
                }
            }
        }
        if must.is_empty() {
            // Only punctuation in the box.
            return Vec::new();
        }
        if let Some(folders) = &scope.folders {
            let any: Vec<(Occur, Box<dyn Query>)> = folders
                .iter()
                .map(|id| (Occur::Should, Box::new(TermQuery::new(Term::from_field_text(f.folder, id), IndexRecordOption::Basic)) as Box<dyn Query>))
                .collect();
            if any.is_empty() {
                return Vec::new();
            }
            must.push((Occur::Must, Box::new(BooleanQuery::new(any))));
        }
        let query = BooleanQuery::new(must);
        let searcher = self.reader.searcher();
        let limit = scope.limit.max(1);
        // More than asked, as the push for recent mail and known people reorders them.
        let Ok(top) = searcher.search(&query, &TopDocs::with_limit(limit * 2).order_by_score()) else { return Vec::new() };
        let mut hits: Vec<(Hit, DocAddress)> = top
            .into_iter()
            .filter_map(|(score, address)| {
                let doc = searcher.doc::<TantivyDocument>(address).ok()?;
                let header = self.header_of(&doc)?;
                let age_days = ((scope.now - header.date).max(0) as f32) / 86_400.0;
                let recent = 1.0 + 0.5 * (-age_days / 120.0).exp();
                let known = if scope.known.contains(&header.from.email().to_lowercase()) { 1.25 } else { 1.0 };
                Some((Hit { header, score: score * recent * known, snippet: None }, address))
            })
            .collect();
        hits.sort_by(|a, b| b.0.score.total_cmp(&a.0.score));
        hits.truncate(limit);
        if all_forms.is_empty() {
            return hits.into_iter().map(|(h, _)| h).collect();
        }
        // Snippets for the hits near the top: what a person reads before refining the search.
        for (hit, address) in hits.iter_mut().take(100) {
            let Ok(doc) = searcher.doc::<TantivyDocument>(*address) else { continue };
            let Some(text) = doc.get_first(f.text).and_then(|v| v.as_str()) else { continue };
            hit.snippet = snippet(&mut self.fold, &mut self.stem_nl, &mut self.stem_en, text, &all_forms);
        }
        hits.into_iter().map(|(h, _)| h).collect()
    }
}

/// A search word as the index sees it.
#[derive(Clone, Debug)]
struct Form {
    folded: String,
    nl: String,
    en: String,
}

/// How many typing errors a word of this length may have and still match.
fn typos(word: &str) -> u8 {
    match word.chars().count() {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// One search word: the word itself in any field (the sender and subject weigh most), its
/// stems, the word with a typing error or two, and, while it is being typed, any word that
/// starts with it.
fn word_query(f: Fields, form: &Form, prefix: bool) -> BooleanQuery {
    let term = |field: Field, text: &str| Term::from_field_text(field, text);
    let exact = |field: Field, boost: f32| -> (Occur, Box<dyn Query>) {
        (Occur::Should, Box::new(BoostQuery::new(Box::new(TermQuery::new(term(field, &form.folded), IndexRecordOption::WithFreqs)), boost)))
    };
    let mut any: Vec<(Occur, Box<dyn Query>)> =
        vec![exact(f.subject, 3.0), exact(f.from, 3.0), exact(f.to, 2.0), exact(f.text, 1.0), exact(f.attachments, 1.5)];
    for (field, stem) in [(f.stem_nl, &form.nl), (f.stem_en, &form.en)] {
        any.push((Occur::Should, Box::new(BoostQuery::new(Box::new(TermQuery::new(term(field, stem), IndexRecordOption::WithFreqs)), 0.8))));
    }
    let distance = typos(&form.folded);
    for field in [f.subject, f.from, f.to, f.text, f.attachments] {
        if prefix && !form.folded.is_empty() {
            any.push((Occur::Should, Box::new(BoostQuery::new(Box::new(FuzzyTermQuery::new_prefix(term(field, &form.folded), 0, true)), 0.5))));
        }
        if distance > 0 {
            any.push((Occur::Should, Box::new(BoostQuery::new(Box::new(FuzzyTermQuery::new(term(field, &form.folded), distance, true)), 0.3))));
        }
    }
    BooleanQuery::new(any)
}

/// The first `max` bytes of `text`, cut at a character boundary.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Edit distance with swapped neighbours counted as one edit, as the index counts it.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[a.len()][b.len()]
}

/// Whether a word of a message is one the search asked for.
fn matches(word: &str, nl: &str, en: &str, form: &Form, prefix: bool) -> bool {
    word == form.folded
        || nl == form.nl
        || en == form.en
        || (prefix && word.starts_with(&form.folded))
        || (typos(&form.folded) > 0 && distance(word, &form.folded) <= usize::from(typos(&form.folded)))
}

/// A piece of `text` around the first word the search asked for, with every such word in it
/// marked; `None` when the text holds none of them (the hit was on the sender, say).
fn snippet(fold: &mut TextAnalyzer, nl: &mut TextAnalyzer, en: &mut TextAnalyzer, text: &str, forms: &[(Form, bool)]) -> Option<Snippet> {
    let scan = cut(text, SNIPPET_SCAN);
    let words = tokens(fold, scan);
    let mut found: Vec<Range<usize>> = Vec::new();
    for (word, range) in words {
        let s_nl = tokens(nl, &word).into_iter().next().map(|x| x.0).unwrap_or_default();
        let s_en = tokens(en, &word).into_iter().next().map(|x| x.0).unwrap_or_default();
        if forms.iter().any(|(form, prefix)| matches(&word, &s_nl, &s_en, form, *prefix)) {
            found.push(range);
            if found.len() > 40 {
                break;
            }
        }
    }
    let first = found.first()?.clone();
    // Start a little before the first word, at the beginning of a word.
    let mut start = first.start.saturating_sub(40);
    while start > 0 && !scan.is_char_boundary(start) {
        start -= 1;
    }
    if start > 0
        && let Some(space) = scan[start..first.start].find(char::is_whitespace)
    {
        start += space + 1;
    }
    let mut end = (start + SNIPPET_CHARS).min(scan.len());
    while end < scan.len() && !scan.is_char_boundary(end) {
        end += 1;
    }
    if end < scan.len()
        && let Some(space) = scan[start.max(first.end).min(end)..end].rfind(char::is_whitespace)
    {
        end = start.max(first.end).min(end) + space;
    }
    let end = end.max(first.end);
    // Line breaks become spaces, byte for byte, so the marks stay where they are.
    let piece: String = scan[start..end].chars().map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect();
    let marks = found.into_iter().filter(|r| r.start >= start && r.end <= end).map(|r| r.start - start..r.end - start).collect();
    Some(Snippet { text: piece, marks })
}

#[cfg(test)]
mod tests {
    use super::*;
    use merel_model::{Address, Flags, Importance, Sealed};

    fn header(id: u32, from: &str, subject: &str, date: i64) -> MessageHeader {
        MessageHeader {
            id: format!("a/INBOX#{id}"),
            folder_id: "a/INBOX".into(),
            remote_id: id.to_string(),
            from: Address::new(Some(from), &format!("{}@voorbeeld.nl", from.to_lowercase().replace(' ', "."))).unwrap(),
            to: vec![Address::new(Some("Alain"), "alain@voorbeeld.nl").unwrap()],
            subject: subject.into(),
            date,
            preview: String::new(),
            flags: Flags::default(),
            has_attachments: false,
            message_id: None,
            categories: Vec::new(),
            importance: Importance::Normal,
            chat: None,
            sealed: Sealed::No,
        }
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn ids(hits: &[Hit]) -> Vec<String> {
        hits.iter().map(|h| h.header.remote_id.clone()).collect()
    }

    fn scope() -> Scope {
        Scope { folders: None, limit: 50, now: 1_000_000_000, known: HashSet::new() }
    }

    fn sample() -> Index {
        let mut i = Index::in_memory().unwrap();
        i.add_text("a", &header(1, "Sanne de Vries", "Offerte drukwerk", 999_000_000), "Hierbij de facturen voor het café in Utrecht.", &[]).unwrap();
        i.add_text(
            "a",
            &header(2, "Jan Bakker", "Verbouwing keuken", 999_500_000),
            "De aannemer begint maandag met de nieuwe keuken.",
            &["plattegrond.pdf".into()],
        )
        .unwrap();
        i.add_header("a", &header(3, "Piet", "Invoices for October", 999_900_000)).unwrap();
        i.commit_if_due(true).unwrap();
        i
    }

    #[test]
    fn words_are_found_in_any_form_without_accents_and_with_a_typo() {
        let mut i = sample();
        let s = scope();
        let find = |i: &mut Index, w: &[&str]| ids(&i.search(Terms { words: &words(w), ..Default::default() }, &s, false));
        assert_eq!(find(&mut i, &["factuur"]), ["1"], "a Dutch stem: factuur finds facturen");
        assert_eq!(find(&mut i, &["invoice"]), ["3"], "an English stem, and the subject of a message without text");
        assert_eq!(find(&mut i, &["cafe"]), ["1"], "accents do not matter");
        assert_eq!(find(&mut i, &["Café"]), ["1"]);
        assert_eq!(find(&mut i, &["aanemer"]), ["2"], "one letter missing");
        assert_eq!(find(&mut i, &["keuken", "maandag"]), ["2"], "every word must be there");
        assert!(find(&mut i, &["keuken", "utrecht"]).is_empty());
        assert_eq!(find(&mut i, &["plattegrond"]), ["2"], "an attachment's name counts");
        assert_eq!(find(&mut i, &["sanne"]), ["1"], "the sender counts");
        assert!(find(&mut i, &["kat"]).is_empty(), "a short word must be exact");
    }

    #[test]
    fn the_word_being_typed_counts_as_the_start_of_a_word() {
        let mut i = sample();
        let typed = i.search(Terms { words: &words(&["verbou"]), ..Default::default() }, &scope(), true);
        assert_eq!(ids(&typed), ["2"]);
        let done = i.search(Terms { words: &words(&["verbou"]), ..Default::default() }, &scope(), false);
        assert!(done.is_empty(), "after Enter a word stands as it is");
    }

    #[test]
    fn operators_hold_to_their_field_and_scope_to_folders() {
        let mut i = sample();
        let from = words(&["sanne"]);
        assert_eq!(ids(&i.search(Terms { from: &from, ..Default::default() }, &scope(), false)), ["1"]);
        let subject = words(&["keuken"]);
        assert_eq!(ids(&i.search(Terms { subject: &subject, ..Default::default() }, &scope(), false)), ["2"]);
        let text_only = words(&["aannemer"]);
        assert!(i.search(Terms { subject: &text_only, ..Default::default() }, &scope(), false).is_empty());
        let elsewhere = Scope { folders: Some(vec!["a/Archief".into()]), ..scope() };
        assert!(i.search(Terms { words: &words(&["keuken"]), ..Default::default() }, &elsewhere, false).is_empty());
        let here = Scope { folders: Some(vec!["a/INBOX".into()]), ..scope() };
        assert_eq!(i.search(Terms { words: &words(&["keuken"]), ..Default::default() }, &here, false).len(), 1);
    }

    #[test]
    fn subject_and_sender_rank_above_the_text_and_known_people_rise() {
        let mut i = Index::in_memory().unwrap();
        i.add_text("a", &header(1, "Kees", "Over iets anders", 999_000_000), "We moeten het nog over de begroting hebben.", &[]).unwrap();
        i.add_text("a", &header(2, "Lies", "Begroting 2027", 999_000_000), "Zie bijlage.", &[]).unwrap();
        i.commit_if_due(true).unwrap();
        let w = words(&["begroting"]);
        assert_eq!(ids(&i.search(Terms { words: &w, ..Default::default() }, &scope(), false)), ["2", "1"]);
        let mut known = scope();
        known.known.insert("kees@voorbeeld.nl".into());
        let mut i2 = Index::in_memory().unwrap();
        i2.add_text("a", &header(1, "Kees", "Begroting", 999_000_000), "", &[]).unwrap();
        i2.add_text("a", &header(2, "Lies", "Begroting", 999_000_000), "", &[]).unwrap();
        i2.commit_if_due(true).unwrap();
        assert_eq!(ids(&i2.search(Terms { words: &w, ..Default::default() }, &known, false)).first().map(String::as_str), Some("1"));
    }

    #[test]
    fn a_snippet_shows_the_words_found_in_the_text() {
        let mut i = sample();
        let hits = i.search(Terms { words: &words(&["aannemer"]), ..Default::default() }, &scope(), false);
        let snip = hits[0].snippet.clone().expect("a snippet");
        assert!(snip.text.contains("aannemer"), "{snip:?}");
        let marked: Vec<&str> = snip.marks.iter().map(|r| &snip.text[r.clone()]).collect();
        assert_eq!(marked, ["aannemer"]);
        let hits = i.search(Terms { words: &words(&["factuur", "cafe"]), ..Default::default() }, &scope(), false);
        let snip = hits[0].snippet.clone().expect("a snippet");
        let marked: Vec<&str> = snip.marks.iter().map(|r| &snip.text[r.clone()]).collect();
        assert_eq!(marked, ["facturen", "café"], "stems and accents are marked too");
        let by_sender = i.search(Terms { words: &words(&["sanne"]), ..Default::default() }, &scope(), false);
        assert_eq!(by_sender[0].snippet, None, "nothing in the text to show");
    }

    #[test]
    fn a_header_is_added_once_and_text_replaces_it_and_deletes_hold_before_a_commit() {
        let mut i = Index::in_memory().unwrap();
        let h = header(7, "Sanne", "Hallo", 5);
        assert!(i.add_header("a", &h).unwrap());
        assert!(!i.add_header("a", &h).unwrap(), "already there, even before a commit");
        i.commit_if_due(true).unwrap();
        assert_eq!(i.counts(), Counts { messages: 1, with_text: 0 });
        assert_eq!(i.without_text("a", 0, 10).iter().map(|h| h.id.clone()).collect::<Vec<_>>(), ["a/INBOX#7"]);
        assert!(i.without_text("a", 6, 10).is_empty(), "older than asked");
        assert!(i.without_text("b", 0, 10).is_empty(), "another account");
        i.add_text("a", &h, "tekst", &[]).unwrap();
        assert!(i.without_text("a", 0, 10).is_empty(), "written, though not committed yet");
        i.commit_if_due(true).unwrap();
        assert_eq!(i.counts(), Counts { messages: 1, with_text: 1 });
        assert!(i.has_text("a/INBOX#7"));
        assert_eq!(i.stored("a/INBOX#7").map(|h| h.subject), Some("Hallo".into()));
        assert_eq!(i.ids_in_folder("a/INBOX", 0), ["a/INBOX#7"]);
        i.delete("a/INBOX#7");
        assert!(!i.contains("a/INBOX#7"));
        i.commit_if_due(true).unwrap();
        assert_eq!(i.counts().messages, 0);
        i.add_header("a", &h).unwrap();
        i.add_header("b", &header(8, "Jan", "Hoi", 5)).unwrap();
        i.commit_if_due(true).unwrap();
        i.delete_account("a");
        i.commit_if_due(true).unwrap();
        assert_eq!(i.counts().messages, 1);
        i.clear().unwrap();
        assert_eq!(i.counts().messages, 0);
    }

    /// A sealer for the tests: a fixed XOR, with the name inside so a renamed file fails.
    struct Xor;

    impl Sealer for Xor {
        fn seal(&self, name: &str, plain: &[u8]) -> std::io::Result<Vec<u8>> {
            let mut out = format!("{name}\0").into_bytes();
            out.extend(plain.iter().map(|b| b ^ 0x5a));
            Ok(out)
        }
        fn open(&self, name: &str, sealed: Vec<u8>) -> Option<Vec<u8>> {
            let rest = sealed.strip_prefix(format!("{name}\0").as_bytes())?;
            Some(rest.iter().map(|b| b ^ 0x5a).collect())
        }
    }

    #[test]
    fn the_index_is_sealed_on_disk_and_comes_back() {
        let dir = std::env::temp_dir().join(format!("merel-search-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut i = Index::open(&dir, Arc::new(Xor)).unwrap();
            i.add_text("a", &header(1, "Sanne", "Geheim plan", 5), "het wachtwoord van de kluis", &[]).unwrap();
            i.commit_if_due(true).unwrap();
        }
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            assert_eq!(path.extension().and_then(|e| e.to_str()), Some("sealed"), "{}", path.display());
            let bytes = std::fs::read(&path).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("kluis"), "nothing readable on disk");
        }
        let mut again = Index::open(&dir, Arc::new(Xor)).unwrap();
        assert_eq!(again.counts().messages, 1);
        assert_eq!(ids(&again.search(Terms { words: &words(&["kluis"]), ..Default::default() }, &scope(), false)), ["1"]);
        // A file that does not open throws the index away rather than half reading it.
        let any = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        std::fs::write(&any, b"rommel").unwrap();
        let fresh = Index::open(&dir, Arc::new(Xor)).unwrap();
        assert_eq!(fresh.counts().messages, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typing_errors_are_counted_as_the_index_counts_them() {
        assert_eq!(distance("factuur", "fatcuur"), 1, "two letters swapped");
        assert_eq!(distance("aannemer", "aanemer"), 1);
        assert_eq!(distance("kat", "hond"), 4);
        assert_eq!(typos("kat"), 0);
        assert_eq!(typos("keuken"), 1);
        assert_eq!(typos("verbouwing"), 2);
        assert_eq!(cut("café", 4), "caf", "never inside a character");
    }
}
