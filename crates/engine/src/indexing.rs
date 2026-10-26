//! Keeping the search index ([`merel_search`]) in step with the mail, and asking it.
//!
//! Everything the session lists goes into the index as it arrives, and an opened message adds
//! its text. Behind that, the index goes through all mail on its own, quietly: for each
//! account one job at a time, so whatever the person asks for waits at most for one small
//! fetch. First it pages through every folder's headers (newest first, as far back as the
//! settings say), then it fetches the text of the messages it knows only by their header,
//! newest first, a few at a time. Once a day each folder is paged through again, which is also
//! when messages that left the server leave the index.
//!
//! The text inside attachments (PDF, Office, plain text; see [`merel_extract`]) is read in the
//! background only, by the crawl: opening a message indexes its own text at once and leaves
//! the attachments to the next fetch, so reading one never holds up the window.
//!
//! The text of a message that came encrypted stays out of the index: what was sent end to end
//! encrypted is not kept readable by a search, even sealed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use merel_mailbox::RemoteHeader;
use merel_model::{FolderKind, MessageHeader};
use merel_search::{Attached, Index, Scope, Sealer, Snippet, Terms};
use merel_store::Store;
use serde::{Deserialize, Serialize};

use crate::meaning::{Meaning, MeaningStatus};
use crate::{AccountStatus, Event, Job, SAMPLE_ACCOUNT, Session, config, message_of};

/// How many headers one job lists.
pub const PAGE: u32 = 200;
/// How many messages one job fetches the text of.
const TEXTS_PER_JOB: usize = 5;
/// How long after a full pass a folder is paged through again.
const RECRAWL: i64 = 24 * 3600;
/// How long an account rests after a job for the index failed.
const PAUSE: Duration = Duration::from_secs(60);
/// A message larger than this is indexed by its header only; its text comes when it is opened.
const BIG: u32 = 10 * 1024 * 1024;
/// How often the index looks for work while nothing ended.
const PLAN_EVERY: Duration = Duration::from_secs(2);
/// The most hits one search lists.
pub const MAX_HITS: usize = 300;
/// Name of the sealed file holding how far the index got.
const STATE_FILE: &str = "crawl.state";

/// What the person chose for the index ("Bestand > Zoeken").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexSettings {
    /// Whether Merel keeps an index at all.
    pub enabled: bool,
    /// How many years back the index fetches the text of messages; `None` for all mail.
    pub years: Option<u32>,
    /// Search on meaning ([`crate::meaning`]): off until the person switches it on, since it
    /// downloads a model.
    pub meaning: bool,
}

impl Default for IndexSettings {
    fn default() -> Self {
        IndexSettings { enabled: true, years: None, meaning: false }
    }
}

/// How far a folder's headers have been paged through.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct FolderCrawl {
    /// Where the next page starts, counted from the newest message.
    offset: u32,
    /// When the last full pass ended, in seconds since the epoch.
    done_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CrawlState {
    folders: HashMap<String, FolderCrawl>,
}

/// What the index holds, for "Bestand > Zoeken".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub messages: u64,
    pub with_text: u64,
    /// Bytes on disk.
    pub size: u64,
    /// Whether the index is still fetching mail.
    pub busy: bool,
}

/// What the index found for the search box as it stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexHits {
    /// The search box's words, scope and folder this answer is for.
    pub query: String,
    pub all: bool,
    pub folder: Option<String>,
    /// How well each message matched.
    pub scores: HashMap<String, f32>,
    /// A piece of each hit's text around the words found.
    pub snippets: HashMap<String, Snippet>,
    /// Hits found on meaning alone, without the words.
    pub related: HashSet<String>,
}

/// The offline cache's key, sealing the index files.
struct StoreSealer(Arc<Store>);

impl Sealer for StoreSealer {
    fn seal(&self, name: &str, plain: &[u8]) -> std::io::Result<Vec<u8>> {
        self.0.seal_blob(&format!("search/{name}"), plain).map_err(std::io::Error::other)
    }

    fn open(&self, name: &str, sealed: Vec<u8>) -> Option<Vec<u8>> {
        self.0.open_blob(&format!("search/{name}"), sealed).map(|p| p.to_vec())
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Default)]
pub struct Indexer {
    index: Option<Index>,
    /// Where the index is sealed; `None` for one in memory (tests).
    dir: Option<PathBuf>,
    sealer: Option<Arc<dyn Sealer>>,
    settings_path: Option<PathBuf>,
    settings: IndexSettings,
    state: CrawlState,
    state_dirty: bool,
    /// Accounts with a job for the index under way.
    busy: HashSet<String>,
    /// Accounts resting after a failed job, until then.
    paused: HashMap<String, Instant>,
    /// For a pass that began at the top of a folder in this run: every id it saw, so what the
    /// server no longer has can leave the index at the end.
    seen: HashMap<String, HashSet<String>>,
    /// What the last search was for, with the index generation it saw.
    searched: Option<(String, bool, bool, Option<String>, u64)>,
    /// When the next jobs were last looked for, and whether a job just ended.
    last_plan: Option<Instant>,
    kick: bool,
    /// Search on meaning, when switched on, and where its model lives.
    meaning: Option<Meaning>,
    model_dir: Option<PathBuf>,
}

impl Indexer {
    /// No index: sample mail, and when the keychain refuses.
    pub fn none() -> Indexer {
        Indexer::default()
    }

    /// An index in memory only, for tests.
    pub fn in_memory() -> Indexer {
        Indexer { index: Index::in_memory().ok(), ..Indexer::default() }
    }

    /// The index next to the accounts file, sealed with the offline cache's key.
    pub fn open(accounts_path: Option<&Path>, store: Option<Arc<Store>>) -> Indexer {
        let (Some(path), Some(store)) = (accounts_path, store) else { return Indexer::none() };
        let settings: IndexSettings = config::load_json(&config::search_path(path));
        let sealer: Arc<dyn Sealer> = Arc::new(StoreSealer(store));
        let mut me = Indexer {
            dir: Some(config::index_dir(path)),
            sealer: Some(sealer),
            settings_path: Some(config::search_path(path)),
            settings,
            model_dir: Some(config::model_dir(path)),
            ..Indexer::default()
        };
        if me.settings.enabled {
            me.start();
        }
        me
    }

    /// Starts search on meaning when it is switched on and the index runs.
    fn start_meaning(&mut self) {
        if self.meaning.is_some() || !self.settings.meaning || self.index.is_none() {
            return;
        }
        let Some(model_dir) = &self.model_dir else { return };
        self.meaning = Some(Meaning::start(model_dir.clone(), self.dir.clone(), self.sealer.clone(), true));
    }

    /// Switches search on meaning off: the model and the vectors go.
    fn stop_meaning(&mut self) -> Result<(), String> {
        match self.meaning.take() {
            Some(m) => m.remove(),
            None => match &self.model_dir {
                Some(dir) => merel_semantic::setup::remove(dir).map_err(|e| e.to_string()),
                None => Ok(()),
            },
        }
    }

    /// Where search on meaning is, for "Bestand > Zoeken"; `None` while it is off.
    pub fn meaning_status(&self) -> Option<MeaningStatus> {
        self.meaning.as_ref().map(Meaning::status)
    }

    /// Tries downloading the model again.
    pub fn retry_meaning(&mut self) {
        if let Some(m) = &mut self.meaning {
            m.retry();
        }
    }

    /// For tests: search on meaning with the model in `dir`, the vectors in memory.
    #[cfg(test)]
    pub(crate) fn with_meaning(mut self, dir: PathBuf) -> Indexer {
        self.settings.meaning = true;
        self.meaning = Some(Meaning::start(dir, None, None, false));
        self
    }

    fn start(&mut self) {
        let (Some(dir), Some(sealer)) = (&self.dir, &self.sealer) else { return };
        match Index::open(dir, Arc::clone(sealer)) {
            Ok(index) => {
                self.state = load_state(dir, sealer.as_ref());
                self.index = Some(index);
            }
            Err(e) => log::warn!("search index disabled: {e}"),
        }
        self.start_meaning();
    }

    pub fn is_on(&self) -> bool {
        self.index.is_some()
    }

    pub fn settings(&self) -> &IndexSettings {
        &self.settings
    }

    /// Applies new settings: switching off throws the index away; fewer years leaves what is
    /// there and fetches no older text.
    pub fn set_settings(&mut self, settings: IndexSettings) -> Result<(), String> {
        let was_on = self.settings.enabled;
        let meaning_was = self.settings.meaning && was_on;
        self.settings = settings;
        if let Some(path) = &self.settings_path {
            config::save_json(path, &self.settings).map_err(|e| e.to_string())?;
        }
        if was_on && !self.settings.enabled {
            self.wipe()?;
        } else if !was_on && self.settings.enabled && self.index.is_none() {
            self.start();
        }
        let meaning_on = self.settings.meaning && self.settings.enabled;
        if meaning_was && !meaning_on {
            self.stop_meaning()?;
        } else if meaning_on {
            self.start_meaning();
        }
        Ok(())
    }

    /// Empties the index and forgets how far it got; with the index on, it starts again.
    pub fn clear(&mut self) -> Result<(), String> {
        let on = self.settings.enabled;
        self.wipe()?;
        if on {
            match &self.dir {
                Some(_) => self.start(),
                None => self.index = Index::in_memory().ok(),
            }
        }
        Ok(())
    }

    fn wipe(&mut self) -> Result<(), String> {
        if let Some(m) = &mut self.meaning {
            m.clear();
        }
        self.index = None;
        self.state = CrawlState::default();
        self.state_dirty = false;
        self.seen.clear();
        self.busy.clear();
        self.searched = None;
        if let Some(dir) = &self.dir {
            merel_search::sealed::wipe(dir).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn progress(&self) -> Progress {
        let Some(index) = &self.index else { return Progress::default() };
        let c = index.counts();
        Progress { messages: c.messages, with_text: c.with_text, size: index.size(), busy: !self.busy.is_empty() }
    }

    /// The oldest date whose text the index fetches.
    fn since(&self) -> i64 {
        match self.settings.years {
            Some(y) => now() - i64::from(y) * 365 * 86_400,
            None => i64::MIN,
        }
    }

    /// Takes note of what the worker reported, before the session applies it.
    pub fn note_event(&mut self, s: &Session, e: &Event) {
        let Some(index) = &mut self.index else {
            if let Event::IndexHeaders { account_id, .. } | Event::IndexTexts { account_id, .. } = e {
                self.busy.remove(account_id);
            }
            return;
        };
        match e {
            Event::Headers { folder_id, headers, offset, limit } => {
                let Some(account) = s.folder(folder_id).map(|f| f.account_id.clone()) else { return };
                for h in headers {
                    if let Some(m) = message_of(folder_id, h.clone())
                        && let Err(e) = index.add_header(&account, &m)
                    {
                        log::warn!("search index: {e}");
                    }
                }
                // A listing of the whole folder shows what is gone from it.
                if *offset == 0 && (headers.len() as u32) < (*limit).max(1) {
                    let listed: HashSet<String> = headers.iter().map(|h| format!("{folder_id}#{}", h.id)).collect();
                    for id in index.ids_in_folder(folder_id, i64::MIN) {
                        if !listed.contains(&id) {
                            index.delete(&id);
                        }
                    }
                }
            }
            Event::Body { message_id, body, .. } => {
                let Some(m) = s.message(message_id) else { return };
                let Some(account) = s.folder(&m.folder_id).map(|f| f.account_id.clone()) else { return };
                // A message does not change; once its text is in, opening it again adds nothing.
                if index.has_text(message_id) {
                    return;
                }
                let encrypted = body.encrypted.is_some() || m.sealed != merel_model::Sealed::No;
                let text = if encrypted { "" } else { body.text.as_str() };
                let attachments: Vec<Attached> =
                    if encrypted { Vec::new() } else { body.attachments.iter().map(|a| Attached::named(&a.name)).collect() };
                // Attachments with text to read wait for the crawl, off this thread.
                let readable = !encrypted && body.attachments.iter().any(|a| merel_extract::kind(&a.name, &a.mime, &a.bytes).is_some());
                let result =
                    if readable { index.add_opened(&account, m, text, &attachments) } else { index.add_text(&account, m, text, &attachments) };
                match result {
                    Err(e) => log::warn!("search index: {e}"),
                    Ok(()) if !readable => {
                        if let Some(meaning) = &mut self.meaning {
                            meaning.note_text(message_id);
                        }
                    }
                    Ok(()) => {}
                }
            }
            Event::Gone { folder_id, message_id, .. } => {
                if let Some(id) = message_id {
                    index.delete(id);
                    if let Some(meaning) = &mut self.meaning {
                        meaning.forget(id);
                    }
                }
                if let Some(f) = folder_id {
                    index.delete_folder(f);
                    self.state.folders.remove(f);
                    self.state_dirty = true;
                    if let Some(meaning) = &mut self.meaning {
                        let prefix = format!("{f}#");
                        meaning.forget_where(|id| id.starts_with(&prefix));
                    }
                }
            }
            Event::IndexHeaders { account_id, folder_id, offset, limit, result } => {
                self.busy.remove(account_id);
                self.kick = true;
                let crawl = self.state.folders.entry(folder_id.clone()).or_default();
                self.state_dirty = true;
                let headers = match result {
                    Ok(h) => h,
                    Err(what) => {
                        // Try this folder again tomorrow, and the others after a rest.
                        log::info!("{account_id}: search index skips {folder_id} for now: {what}");
                        *crawl = FolderCrawl { offset: 0, done_at: Some(now()) };
                        self.seen.remove(folder_id);
                        self.paused.insert(account_id.clone(), Instant::now() + PAUSE);
                        return;
                    }
                };
                let since = match self.settings.years {
                    Some(y) => now() - i64::from(y) * 365 * 86_400,
                    None => i64::MIN,
                };
                for h in headers {
                    let Some(m) = message_of(folder_id, h.clone()) else { continue };
                    if let Some(seen) = self.seen.get_mut(folder_id) {
                        seen.insert(m.id.clone());
                    }
                    let result = if index.contains(&m.id) {
                        Ok(())
                    } else if h.size > BIG || m.date < since {
                        // Too large to fetch in passing, or older than asked: the header will do.
                        index.add_text(account_id, &m, "", &[])
                    } else {
                        index.add_header(account_id, &m).map(|_| ())
                    };
                    if let Err(e) = result {
                        log::warn!("search index: {e}");
                    }
                }
                let oldest = headers.iter().map(|h| h.date).min().unwrap_or(i64::MIN);
                let complete = (headers.len() as u32) < (*limit).max(1) || oldest < since;
                if !complete {
                    *crawl = FolderCrawl { offset: offset + limit, done_at: crawl.done_at };
                    return;
                }
                *crawl = FolderCrawl { offset: 0, done_at: Some(now()) };
                if let Some(seen) = self.seen.remove(folder_id) {
                    let from = if (headers.len() as u32) < (*limit).max(1) { i64::MIN } else { oldest };
                    for id in index.ids_in_folder(folder_id, from) {
                        if !seen.contains(&id) {
                            index.delete(&id);
                        }
                    }
                }
            }
            Event::IndexTexts { account_id, texts, failed } => {
                self.busy.remove(account_id);
                self.kick = true;
                for t in texts {
                    let header = s.message(&t.message_id).cloned().or_else(|| index.stored(&t.message_id));
                    match (t.gone, header) {
                        (true, _) | (false, None) => index.delete(&t.message_id),
                        (false, Some(h)) => match index.add_text(account_id, &h, &t.text, &t.attachments) {
                            Err(e) => log::warn!("search index: {e}"),
                            Ok(()) => {
                                if let Some(meaning) = &mut self.meaning {
                                    meaning.note_text(&t.message_id);
                                }
                            }
                        },
                    }
                }
                if let Some((what, first)) = failed {
                    // One message that keeps failing must not stop the rest: it goes without text.
                    log::info!("{account_id}: search index could not fetch a message: {what}");
                    if let Some(h) = s.message(first).cloned().or_else(|| index.stored(first)) {
                        let _ = index.add_text(account_id, &h, "", &[]);
                    }
                    self.paused.insert(account_id.clone(), Instant::now() + PAUSE);
                }
            }
            _ => {}
        }
    }

    /// Messages that left their folder in the session (deleted, moved): their place in the
    /// index is gone; a moved message comes back under its new folder when that is listed.
    pub fn forget(&mut self, ids: &[String]) {
        if let Some(index) = &mut self.index {
            for id in ids {
                index.delete(id);
                if let Some(meaning) = &mut self.meaning {
                    meaning.forget(id);
                }
            }
        }
    }

    pub fn remove_account(&mut self, account_id: &str) {
        if let Some(index) = &mut self.index {
            index.delete_account(account_id);
        }
        if let Some(meaning) = &mut self.meaning {
            let prefix = format!("{account_id}/");
            meaning.forget_where(|id| id.starts_with(&prefix));
        }
        self.state.folders.retain(|f, _| !f.starts_with(&format!("{account_id}/")));
        self.state_dirty = true;
    }

    /// Writes what changed when it is time (or now, with `force`), and says which jobs the
    /// index needs next: at most one per connected account that has none under way.
    pub fn tick(&mut self, s: &Session, force: bool) -> Vec<Job> {
        let Some(index) = &mut self.index else { return Vec::new() };
        let written = match index.commit_if_due(force) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("search index not written: {e}");
                false
            }
        };
        if let Some(meaning) = &mut self.meaning
            && meaning.tick(index, force)
        {
            // The search's words came back from the model: the answer may change.
            self.searched = None;
        }
        if self.state_dirty && (written || force) {
            self.save_state();
        }
        // Looking for work costs a query of the index; not every frame.
        if !force && !self.kick && self.last_plan.is_some_and(|t| t.elapsed() < PLAN_EVERY) {
            return Vec::new();
        }
        self.kick = false;
        self.last_plan = Some(Instant::now());
        let mut jobs = Vec::new();
        let now_i = Instant::now();
        self.paused.retain(|_, until| *until > now_i);
        for account in &s.accounts {
            let id = &account.id;
            if id == SAMPLE_ACCOUNT || self.busy.contains(id) || self.paused.contains_key(id) || s.status.get(id) != Some(&AccountStatus::Online) {
                continue;
            }
            if let Some(job) = self.next_job(s, id) {
                self.busy.insert(id.clone());
                jobs.push(job);
            }
        }
        jobs
    }

    /// The account's next job for the index: a page of headers while a folder is due, else
    /// the text of the newest messages known by their header only.
    fn next_job(&mut self, s: &Session, account_id: &str) -> Option<Job> {
        let now = now();
        let mut folders: Vec<_> = s.folders.iter().filter(|f| f.account_id == account_id && f.kind != FolderKind::Junk).collect();
        folders.sort_by_key(|f| match f.kind {
            FolderKind::Inbox => 0,
            FolderKind::Sent => 1,
            FolderKind::Archive => 2,
            FolderKind::Other => 3,
            FolderKind::Drafts => 4,
            FolderKind::Deleted => 5,
            FolderKind::Junk => 6,
        });
        for f in &folders {
            let crawl = self.state.folders.get(&f.id).cloned().unwrap_or_default();
            if crawl.done_at.is_some_and(|t| now - t < RECRAWL) {
                continue;
            }
            if crawl.offset == 0 {
                self.seen.insert(f.id.clone(), HashSet::new());
            }
            return Some(Job::IndexHeaders {
                account_id: account_id.to_string(),
                folder_id: f.id.clone(),
                path: f.path.clone(),
                offset: crawl.offset,
                limit: PAGE,
            });
        }
        let since = self.since();
        let index = self.index.as_mut()?;
        let todo = index.without_text(account_id, since, TEXTS_PER_JOB * 4);
        let mut first: Option<(String, String)> = None;
        let mut ids = Vec::new();
        for h in todo {
            let Some(folder) = s.folder(&h.folder_id) else {
                // Its folder went away.
                index.delete(&h.id);
                continue;
            };
            let place = first.get_or_insert_with(|| (folder.id.clone(), folder.path.clone()));
            if place.0 == folder.id && ids.len() < TEXTS_PER_JOB {
                ids.push((h.id.clone(), h.remote_id.clone()));
            }
        }
        let (folder_id, path) = first?;
        Some(Job::IndexTexts { account_id: account_id.to_string(), folder_id, path, ids })
    }

    fn save_state(&mut self) {
        let (Some(dir), Some(sealer)) = (&self.dir, &self.sealer) else {
            self.state_dirty = false;
            return;
        };
        let Ok(plain) = serde_json::to_vec(&self.state) else { return };
        match sealer.seal(STATE_FILE, &plain).and_then(|sealed| std::fs::write(dir.join(STATE_FILE), sealed)) {
            Ok(()) => self.state_dirty = false,
            Err(e) => log::warn!("search index state not written: {e}"),
        }
    }

    /// Asks the index about the search box when its words, scope, folder or the index itself
    /// changed since the last time, and puts the answer in the session: hits it did not list
    /// yet join it (older mail), as server hits do.
    pub fn search(&mut self, s: &mut Session) {
        let Some(index) = &mut self.index else {
            s.index_hits = None;
            return;
        };
        let key = (s.search.clone(), s.search_all, s.search_exact, s.selected_folder.clone(), index.generation());
        if self.searched.as_ref() == Some(&key) {
            return;
        }
        self.searched = Some(key);
        let query = s.search_query();
        let terms = Terms {
            words: &query.words,
            alike: &query.alike,
            either: &query.either,
            phrases: &query.phrases,
            not: &query.not,
            from: &query.from,
            to: &query.to,
            subject: &query.subject,
            attachments: &query.attachment_names,
            after: query.after,
            before: query.before,
            amount: query.amount,
            has: &query.has,
            size: query.size,
        };
        if terms.is_empty() {
            s.index_hits = None;
            return;
        }
        // `map:` and `account:` choose the folders themselves, else the scope menu does.
        let folders = match s.search_folders(&query) {
            Some(places) => Some(places.into_iter().collect()),
            None if s.search_all => None,
            None => s.selected_folder.as_deref().map(|id| s.members(id)),
        };
        let known: HashSet<String> = s.contacts.iter().flat_map(|c| c.emails.iter().map(|e| e.to_lowercase())).collect();
        let scope = Scope { folders, limit: MAX_HITS, now: now(), known };
        // The last word may still be half typed, until a space follows it.
        let typing = !s.search.ends_with(char::is_whitespace);
        let hits = index.search(terms, &scope, typing);
        let mut answer = IndexHits { query: s.search.clone(), all: s.search_all, folder: s.selected_folder.clone(), ..IndexHits::default() };
        let listed: HashSet<String> = s.messages.iter().map(|m| m.id.clone()).collect();
        let mut joined: Vec<MessageHeader> = Vec::new();
        let ids: Vec<String> = hits.iter().map(|h| h.header.id.clone()).collect();
        for hit in hits {
            if !listed.contains(&hit.header.id) && s.folder(&hit.header.folder_id).is_some() {
                joined.push(hit.header.clone());
            }
            answer.scores.insert(hit.header.id.clone(), hit.score);
            if let Some(snip) = hit.snippet {
                answer.snippets.insert(hit.header.id.clone(), snip);
            }
        }
        // Messages about the same thing, also without the words: only for plain words, and held
        // to the search's scope and its other search words.
        let words: Vec<&str> = query.words.iter().chain(&query.phrases).chain(query.either.iter().flatten()).map(String::as_str).collect();
        if let Some(meaning) = &mut self.meaning
            && !words.is_empty()
            && query.not.is_empty()
            && query.attachment_names.is_empty()
            && query.amount.is_none()
            && query.has.is_empty()
        {
            let allowed = |id: &str| {
                index.stored(id).is_some_and(|h| {
                    scope.folders.as_ref().is_none_or(|f| f.contains(&h.folder_id)) && s.folder(&h.folder_id).is_some() && query.keeps(&h)
                })
            };
            let related: Vec<String> = meaning.related(&words.join(" "), allowed).into_iter().map(|(id, _)| id).collect();
            if !related.is_empty() {
                answer.scores = crate::meaning::fuse(&ids, &related).into_iter().collect();
                for id in related {
                    if ids.contains(&id) {
                        continue;
                    }
                    if !listed.contains(&id)
                        && let Some(h) = index.stored(&id)
                    {
                        joined.push(h);
                    }
                    answer.related.insert(id);
                }
            }
        }
        if !joined.is_empty() {
            s.messages.extend(joined);
            s.recount_unread();
        }
        s.index_hits = Some(answer);
    }

    /// Writes everything now; for when the window closes.
    pub fn flush(&mut self, s: &Session) {
        let _ = self.tick(s, true);
        if let Some(meaning) = &mut self.meaning {
            meaning.flush();
        }
    }
}

fn load_state(dir: &Path, sealer: &dyn Sealer) -> CrawlState {
    let Ok(bytes) = std::fs::read(dir.join(STATE_FILE)) else { return CrawlState::default() };
    sealer.open(STATE_FILE, bytes).and_then(|plain| serde_json::from_slice(&plain).ok()).unwrap_or_default()
}

/// The text of a message for the index, as the worker fetched it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexedText {
    pub message_id: String,
    pub text: String,
    pub attachments: Vec<Attached>,
    /// The server no longer has the message.
    pub gone: bool,
}

/// What the index keeps of a raw message: its text and its attachments, with the text inside
/// them where it can be read; nothing of one that came encrypted.
pub fn text_of(raw: &[u8]) -> (String, Vec<Attached>) {
    if merel_pgp::is_encrypted(raw) || merel_smime::is_encrypted(raw) {
        return (String::new(), Vec::new());
    }
    let body = merel_mime::parse_body(raw);
    let attachments = body
        .attachments
        .iter()
        .map(|a| Attached { name: a.name.clone(), text: merel_extract::text(&a.name, &a.mime, &a.bytes).unwrap_or_default() })
        .collect();
    (body.text, attachments)
}

/// For the worker: the headers a page for the index listed.
pub fn headers_event(account_id: &str, folder_id: &str, offset: u32, limit: u32, result: Result<Vec<RemoteHeader>, String>) -> Event {
    Event::IndexHeaders { account_id: account_id.to_string(), folder_id: folder_id.to_string(), offset, limit, result }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, header};
    use merel_mailbox::RemoteFolder;

    fn session() -> Session {
        let mut s = Session::with_accounts(vec![account("a")]);
        let inbox = RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true, parent: None };
        let junk = RemoteFolder { path: "Junk".into(), name: "Junk".into(), kind: FolderKind::Junk, selectable: true, parent: None };
        s.apply(Event::Connected { account_id: "a".into(), folders: vec![inbox, junk], syncs_items: false });
        s.take_jobs();
        s
    }

    fn remote(n: u32, subject: &str, date: i64) -> RemoteHeader {
        let mut h = header(n);
        h.header.subject = subject.into();
        h.date = date;
        h
    }

    fn text(id: &str, text: &str) -> IndexedText {
        IndexedText { message_id: id.into(), text: text.into(), ..IndexedText::default() }
    }

    #[test]
    fn the_index_pages_through_a_folder_then_fetches_the_text_and_a_search_finds_old_mail() {
        let mut s = session();
        let mut ix = Indexer::in_memory();
        let jobs = ix.tick(&s, true);
        assert_eq!(
            jobs,
            [Job::IndexHeaders { account_id: "a".into(), folder_id: "a/INBOX".into(), path: "INBOX".into(), offset: 0, limit: PAGE }],
            "the inbox first, and never the junk folder"
        );
        assert!(ix.tick(&s, true).is_empty(), "one job at a time per account");
        let page = vec![remote(1, "Offerte", now() - 86_400 * 400), remote(2, "Verbouwing", now() - 3_600)];
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Ok(page)));
        let jobs = ix.tick(&s, true);
        let Some(Job::IndexTexts { folder_id, path, ids, .. }) = jobs.first() else { panic!("expected texts, got {jobs:?}") };
        assert_eq!((folder_id.as_str(), path.as_str()), ("a/INBOX", "INBOX"));
        assert_eq!(ids, &[("a/INBOX#2".to_string(), "2".to_string()), ("a/INBOX#1".to_string(), "1".to_string())], "newest first");
        let texts = vec![text("a/INBOX#2", "De aannemer begint maandag."), text("a/INBOX#1", "Hierbij de facturen.")];
        ix.note_event(&s, &Event::IndexTexts { account_id: "a".into(), texts, failed: None });
        assert!(ix.tick(&s, true).is_empty(), "everything has its text");
        assert_eq!(ix.progress(), Progress { messages: 2, with_text: 2, size: 0, busy: false });

        // The session never listed these messages; the index finds them and they join it.
        assert!(s.messages.is_empty());
        s.selected_folder = Some("a/INBOX".into());
        s.search = "factuur".into();
        ix.search(&mut s);
        assert_eq!(s.visible_messages().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a/INBOX#1"]);
        assert!(s.current_index_hits().is_some_and(|h| h.snippets.contains_key("a/INBOX#1")));
        s.search = "aanemer".into();
        ix.search(&mut s);
        assert_eq!(s.visible_messages().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a/INBOX#2"], "with a typing error");
        s.search = "van:y.nl".into();
        ix.search(&mut s);
        assert_eq!(s.visible_messages().len(), 2, "operators go to the index too");
        s.search.clear();
        ix.search(&mut s);
        assert!(s.index_hits.is_none());
    }

    #[test]
    fn hits_rank_best_first_unless_the_person_wants_the_newest() {
        let mut s = session();
        let mut ix = Indexer::in_memory();
        ix.tick(&s, true);
        let page = vec![remote(1, "Begroting 2027", now() - 86_400 * 30), remote(2, "Over iets anders", now() - 60)];
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Ok(page)));
        ix.tick(&s, true);
        let texts = vec![text("a/INBOX#1", "Zie bijlage."), text("a/INBOX#2", "Nog even over de begroting.")];
        ix.note_event(&s, &Event::IndexTexts { account_id: "a".into(), texts, failed: None });
        ix.tick(&s, true);
        s.selected_folder = Some("a/INBOX".into());
        s.search = "begroting ".into();
        ix.search(&mut s);
        let order = |s: &Session| s.visible_messages().iter().map(|m| m.remote_id.clone()).collect::<Vec<_>>();
        assert_eq!(order(&s), ["1", "2"], "the subject weighs more than a newer text");
        s.search_newest = true;
        assert_eq!(order(&s), ["2", "1"]);
    }

    #[test]
    fn a_new_pass_lets_go_of_what_the_server_no_longer_has_and_listings_and_moves_do_too() {
        let s = session();
        let mut ix = Indexer::in_memory();
        ix.tick(&s, true);
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Ok(vec![remote(1, "Een", 1), remote(2, "Twee", 2), remote(3, "Drie", 3)])));
        ix.tick(&s, true);
        assert_eq!(ix.progress().messages, 3);
        // A day later the folder is paged through again; message 2 went.
        ix.state.folders.insert("a/INBOX".into(), FolderCrawl { offset: 0, done_at: Some(now() - RECRAWL - 1) });
        ix.busy.clear();
        let jobs = ix.tick(&s, true);
        assert!(matches!(jobs.first(), Some(Job::IndexHeaders { offset: 0, .. })), "{jobs:?}");
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Ok(vec![remote(1, "Een", 1), remote(3, "Drie", 3)])));
        ix.tick(&s, true);
        assert_eq!(ix.progress().messages, 2);
        // A listing of the whole folder in the session says the same.
        ix.note_event(&s, &Event::Headers { folder_id: "a/INBOX".into(), headers: vec![remote(3, "Drie", 3)], offset: 0, limit: 200 });
        ix.tick(&s, true);
        assert_eq!(ix.progress().messages, 1);
        // A message deleted or moved in Merel leaves the index at once.
        ix.forget(&["a/INBOX#3".to_string()]);
        ix.tick(&s, true);
        assert_eq!(ix.progress().messages, 0);
        // A folder the server lost goes with all its messages.
        ix.note_event(&s, &Event::Headers { folder_id: "a/INBOX".into(), headers: vec![remote(4, "Vier", 4)], offset: 0, limit: 1 });
        ix.note_event(&s, &Event::Gone { account_id: "a".into(), folder_id: Some("a/INBOX".into()), message_id: None });
        ix.tick(&s, true);
        assert_eq!(ix.progress().messages, 0);
    }

    #[test]
    fn a_long_folder_takes_pages_and_old_mail_keeps_to_its_header() {
        let s = session();
        let mut ix = Indexer::in_memory();
        ix.settings.years = Some(1);
        ix.tick(&s, true);
        let page: Vec<RemoteHeader> = (0..PAGE).map(|n| remote(n + 100, "Recent", now() - i64::from(n))).collect();
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Ok(page)));
        let jobs = ix.tick(&s, true);
        assert!(matches!(jobs.first(), Some(Job::IndexHeaders { offset: PAGE, .. })), "the next page: {jobs:?}");
        // A page reaching past a year back ends the pass; that old message needs no text.
        let page = vec![remote(1, "Oud", now() - 2 * 365 * 86_400), remote(2, "Nieuw", now() - 10)];
        ix.note_event(&s, &headers_event("a", "a/INBOX", PAGE, PAGE, Ok(page)));
        let jobs = ix.tick(&s, true);
        let Some(Job::IndexTexts { ids, .. }) = jobs.first() else { panic!("{jobs:?}") };
        assert!(!ids.iter().any(|(id, _)| id == "a/INBOX#1"), "older than a year: header only");
        assert_eq!(ix.progress().messages, PAGE as u64 + 2);
    }

    #[test]
    fn failures_rest_the_account_and_skip_what_keeps_failing() {
        let s = session();
        let mut ix = Indexer::in_memory();
        ix.tick(&s, true);
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Err("nee".into())));
        assert!(ix.tick(&s, true).is_empty(), "the account rests");
        assert!(ix.state.folders.get("a/INBOX").is_some_and(|f| f.done_at.is_some()), "the folder waits until tomorrow");
        ix.paused.clear();
        ix.state.folders.clear();
        ix.tick(&s, true);
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Ok(vec![remote(1, "Een", now()), remote(2, "Twee", now())])));
        ix.tick(&s, true);
        ix.note_event(
            &s,
            &Event::IndexTexts { account_id: "a".into(), texts: vec![text("a/INBOX#2", "tekst")], failed: Some(("stuk".into(), "a/INBOX#1".into())) },
        );
        ix.paused.clear();
        assert!(ix.tick(&s, true).is_empty(), "the one that failed went without text, the other has it");
        assert_eq!(ix.progress().with_text, 2);
    }

    #[test]
    fn an_opened_message_adds_its_text_but_an_encrypted_one_only_its_header() {
        let mut s = session();
        let mut ix = Indexer::in_memory();
        s.apply(Event::Headers { folder_id: "a/INBOX".into(), headers: vec![remote(1, "Plan", 1), remote(2, "Geheim", 2)], offset: 0, limit: 200 });
        let raw = b"From: x@y.nl\r\nSubject: Plan\r\n\r\nDe kluis staat in de kelder.\r\n".to_vec();
        let body = merel_mime::parse_body(&raw);
        let event = Event::Body { message_id: "a/INBOX#1".into(), body: Box::new(body), header: Box::default(), raw, carried: None };
        ix.note_event(&s, &event);
        let pgp = b"From: x@y.nl\r\nContent-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=b\r\n\r\n--b\r\n\r\nVersion: 1\r\n--b--\r\n";
        assert_eq!(text_of(pgp), (String::new(), Vec::new()), "nothing of an encrypted message");
        let mut sealed = merel_mime::parse_body(b"From: x@y.nl\r\n\r\nde code is 1234\r\n");
        sealed.encrypted = Some(merel_mime::Encryption::Opened);
        let event = Event::Body { message_id: "a/INBOX#2".into(), body: Box::new(sealed), header: Box::default(), raw: b"x".to_vec(), carried: None };
        ix.note_event(&s, &event);
        ix.tick(&s, true);
        s.search = "kelder".into();
        s.search_all = true;
        ix.search(&mut s);
        assert_eq!(s.visible_messages().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a/INBOX#1"]);
        s.search = "1234".into();
        ix.search(&mut s);
        assert!(s.current_index_hits().is_some_and(|h| h.scores.is_empty()), "the opened encrypted text stays out");
    }

    #[test]
    fn the_text_inside_an_attachment_is_read_by_the_crawl_not_when_the_message_opens() {
        let mut s = session();
        let mut ix = Indexer::in_memory();
        let page = vec![remote(1, "Offerte", now() - 3_600)];
        s.apply(Event::Headers { folder_id: "a/INBOX".into(), headers: page.clone(), offset: 0, limit: 200 });
        ix.tick(&s, true);
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Ok(page)));
        let raw = b"From: x@y.nl\r\nSubject: Offerte\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nZie bijlage.\r\n--b\r\nContent-Type: text/csv; name=prijzen.csv\r\nContent-Disposition: attachment; filename=prijzen.csv\r\n\r\nartikel;prijs\r\naanrecht graniet;1250\r\n--b--\r\n".to_vec();
        let body = merel_mime::parse_body(&raw);
        assert_eq!(body.attachments.len(), 1);
        ix.note_event(
            &s,
            &Event::Body { message_id: "a/INBOX#1".into(), body: Box::new(body), header: Box::default(), raw: raw.clone(), carried: None },
        );
        let jobs = ix.tick(&s, true);
        let Some(Job::IndexTexts { ids, .. }) = jobs.first() else { panic!("the attachment is still to be read: {jobs:?}") };
        assert_eq!(ids, &[("a/INBOX#1".to_string(), "1".to_string())]);
        let (text, attachments) = text_of(&raw);
        assert_eq!(attachments, [Attached { name: "prijzen.csv".into(), text: "artikel;prijs\naanrecht graniet;1250".into() }]);
        let texts = vec![IndexedText { message_id: "a/INBOX#1".into(), text, attachments, gone: false }];
        ix.note_event(&s, &Event::IndexTexts { account_id: "a".into(), texts, failed: None });
        assert!(ix.tick(&s, true).is_empty());
        s.search = "graniet".into();
        s.search_all = true;
        ix.search(&mut s);
        assert_eq!(s.visible_messages().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a/INBOX#1"]);
        let snip = s.current_index_hits().and_then(|h| h.snippets.get("a/INBOX#1").cloned());
        assert_eq!(snip.and_then(|x| x.attachment), Some("prijzen.csv".to_string()));
    }

    #[test]
    fn switching_off_throws_the_index_away() {
        let s = session();
        let mut ix = Indexer::in_memory();
        ix.note_event(&s, &Event::Headers { folder_id: "a/INBOX".into(), headers: vec![remote(1, "Een", 1)], offset: 0, limit: 200 });
        ix.tick(&s, true);
        assert!(ix.is_on());
        ix.set_settings(IndexSettings { enabled: false, ..IndexSettings::default() }).unwrap();
        assert!(!ix.is_on());
        assert!(ix.tick(&s, true).is_empty());
        ix.clear().unwrap();
        assert!(!ix.is_on(), "clearing does not switch it on");
    }

    #[test]
    fn search_on_meaning_adds_mail_about_the_same_thing_and_switching_it_off_removes_the_model() {
        let dir = std::env::temp_dir().join(format!("merel-meaning-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        merel_semantic::setup::tests_support::prepare(&dir);
        let mut s = session();
        let mut ix = Indexer::in_memory().with_meaning(dir.clone());
        // The tiny test model knows no meaning: let every message count as close.
        if let Some(m) = &mut ix.meaning {
            m.loosen();
        }
        ix.tick(&s, true);
        let page = vec![remote(1, "Offerte", now() - 60), remote(2, "Planning", now() - 120)];
        ix.note_event(&s, &headers_event("a", "a/INBOX", 0, PAGE, Ok(page)));
        ix.tick(&s, true);
        let texts = vec![text("a/INBOX#1", "aannemer factuur"), text("a/INBOX#2", "keuken")];
        ix.note_event(&s, &Event::IndexTexts { account_id: "a".into(), texts, failed: None });
        let read_all = |ix: &mut Indexer, s: &Session| {
            let until = Instant::now() + Duration::from_secs(60);
            while ix.meaning_status().is_some_and(|m| m.vectors < 2) && Instant::now() < until {
                ix.tick(s, true);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        read_all(&mut ix, &s);
        assert_eq!(ix.meaning_status().map(|m| (m.model, m.vectors)), Some((crate::meaning::ModelState::Ready, 2)));

        s.selected_folder = Some("a/INBOX".into());
        s.search = "aannemer ".into();
        ix.search(&mut s);
        let visible = |s: &Session| s.visible_messages().iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        assert_eq!(visible(&s), ["a/INBOX#1"], "the words first, while the model reads the search");
        let until = Instant::now() + Duration::from_secs(60);
        while s.current_index_hits().is_none_or(|h| h.related.is_empty()) && Instant::now() < until {
            ix.tick(&s, true);
            ix.search(&mut s);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(visible(&s), ["a/INBOX#1", "a/INBOX#2"], "the message with the words stays on top");
        assert!(s.current_index_hits().is_some_and(|h| h.related.contains("a/INBOX#2") && !h.related.contains("a/INBOX#1")));
        s.search = "aannemer van:niemand ".into();
        ix.search(&mut s);
        assert!(visible(&s).is_empty(), "the other search words hold for related mail too");

        ix.forget(&["a/INBOX#2".to_string()]);
        assert_eq!(ix.meaning_status().map(|m| m.vectors), Some(1));
        ix.set_settings(IndexSettings { meaning: false, ..IndexSettings::default() }).unwrap();
        assert!(ix.meaning_status().is_none());
        assert!(!merel_semantic::setup::is_ready(&dir), "switching off throws the model away");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
