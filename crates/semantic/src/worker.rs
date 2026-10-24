//! A thread of its own for the model: downloading it, and turning texts into vectors, so the
//! window never waits for either. A search's words go before the messages still to be read.
//! The model is let go after a couple of idle minutes and loaded again when needed, so it
//! only takes memory while there is work.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use crate::model::{Embedder, Kind};
use crate::setup::{self, Stage};

/// How long the model stays loaded without work.
const IDLE: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Download and prepare the model.
    Install,
    /// The vector of a message.
    Embed { id: String, text: String },
    /// The vector of a search's words; only the latest one asked counts.
    Query { text: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Progress(Stage),
    Installed,
    Failed(String),
    Vector { id: String, vector: Vec<f32> },
    QueryVector { text: String, vector: Vec<f32> },
}

pub struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    /// A worker for the model in `dir`. `slow` makes it rest as long as it worked after each
    /// message, so reading all mail never takes the whole computer.
    pub fn start(dir: PathBuf, slow: bool) -> Worker {
        let (tx, requests) = channel::<Request>();
        let (replies, rx) = channel::<Reply>();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancel);
        let spawned = std::thread::Builder::new().name("merel-semantic".into()).spawn(move || run(&dir, &requests, &replies, &stop, slow));
        if let Err(e) = spawned {
            log::warn!("search on meaning: no thread: {e}");
        }
        Worker { tx, rx, cancel }
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }

    /// The next answer, if one is in.
    pub fn try_recv(&self) -> Option<Reply> {
        self.rx.try_recv().ok()
    }

    /// Waits for the next answer, at most `timeout` (tests).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Reply> {
        self.rx.recv_timeout(timeout).ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // A download under way stops; the thread ends once the channel closes.
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn run(dir: &std::path::Path, requests: &Receiver<Request>, replies: &Sender<Reply>, cancel: &AtomicBool, slow: bool) {
    let mut queue: VecDeque<Request> = VecDeque::new();
    let mut embedder: Option<Embedder> = None;
    loop {
        // Everything asked so far, without waiting.
        while let Ok(r) = requests.try_recv() {
            queue.push_back(r);
        }
        if queue.is_empty() {
            match requests.recv_timeout(IDLE) {
                Ok(r) => queue.push_back(r),
                Err(RecvTimeoutError::Timeout) => {
                    embedder = None;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
            continue;
        }
        // The latest search first; earlier ones are out of date.
        let request = match queue.iter().rposition(|r| matches!(r, Request::Query { .. })) {
            Some(at) => {
                let latest = queue.remove(at);
                queue.retain(|r| !matches!(r, Request::Query { .. }));
                latest
            }
            None => queue.pop_front(),
        };
        let Some(request) = request else { continue };
        let reply = match request {
            Request::Install => {
                let report = |stage| {
                    let _ = replies.send(Reply::Progress(stage));
                };
                match setup::install(dir, cancel, &report) {
                    Ok(()) => Reply::Installed,
                    Err(e) => Reply::Failed(e.to_string()),
                }
            }
            Request::Embed { id, text } => {
                let started = Instant::now();
                match model(dir, &mut embedder).and_then(|e| e.embed(&text, Kind::Passage).map_err(|e| e.to_string())) {
                    Ok(vector) => {
                        if slow {
                            std::thread::sleep(started.elapsed().min(Duration::from_secs(2)));
                        }
                        Reply::Vector { id, vector }
                    }
                    Err(e) => {
                        // Without a model the rest of the queue cannot be read either.
                        queue.retain(|r| !matches!(r, Request::Embed { .. }));
                        Reply::Failed(e)
                    }
                }
            }
            Request::Query { text } => match model(dir, &mut embedder).and_then(|e| e.embed(&text, Kind::Query).map_err(|e| e.to_string())) {
                Ok(vector) => Reply::QueryVector { text, vector },
                Err(e) => Reply::Failed(e),
            },
        };
        if replies.send(reply).is_err() {
            return;
        }
    }
}

/// The model, loaded when it is not.
fn model<'a>(dir: &std::path::Path, embedder: &'a mut Option<Embedder>) -> Result<&'a Embedder, String> {
    if embedder.is_none() {
        if !setup::is_ready(dir) {
            return Err("the model is not on this computer".into());
        }
        *embedder = Some(Embedder::load(dir).map_err(|e| e.to_string())?);
    }
    embedder.as_ref().ok_or_else(|| "no model".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_and_searches_come_back_as_vectors_and_a_missing_model_says_so() {
        let dir = std::env::temp_dir().join(format!("merel-semantic-worker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let w = Worker::start(dir.clone(), false);
        w.send(Request::Embed { id: "a".into(), text: "keuken".into() });
        w.send(Request::Embed { id: "b".into(), text: "factuur".into() });
        assert!(matches!(w.recv_timeout(Duration::from_secs(10)), Some(Reply::Failed(_))));
        assert_eq!(w.recv_timeout(Duration::from_millis(300)), None, "the rest of the queue is dropped, not failed one by one");

        crate::setup::tests_support::prepare(&dir);
        w.send(Request::Embed { id: "a".into(), text: "keuken".into() });
        w.send(Request::Query { text: "aannemer".into() });
        let mut got = Vec::new();
        for _ in 0..2 {
            got.push(w.recv_timeout(Duration::from_secs(30)).unwrap());
        }
        assert!(got.iter().any(|r| matches!(r, Reply::Vector { id, vector } if id == "a" && vector.len() == 64)), "{got:?}");
        assert!(got.iter().any(|r| matches!(r, Reply::QueryVector { text, .. } if text == "aannemer")), "{got:?}");
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
