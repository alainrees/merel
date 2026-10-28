//! `merel-signal`: Merel's link to Signal, as a separate program.
//!
//! It links as a secondary device of the person's Signal account (as Signal Desktop does) and
//! speaks JSON lines: one event per line on stdout, one command per line on stdin.
//!
//! ```text
//! merel-signal link <dir> <device name>   → {"type":"link","url":…} then {"type":"linked"}
//! merel-signal run <dir>                   → {"type":"message",…}… and {"type":"synced"}
//!                                          ← {"type":"send","thread":…,"text":…}
//! ```
//!
//! The store in `<dir>` is encrypted with the key in the environment variable `MEREL_SIGNAL_KEY`,
//! which Merel keeps in the system keychain.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use futures::channel::oneshot;
use futures::{StreamExt, future, pin_mut};
use presage::Manager;
use presage::libsignal_service::configuration::SignalServers;
use presage::libsignal_service::content::{Content, ContentBody, DataMessage};
use presage::libsignal_service::protocol::ServiceId;
use presage::manager::Registered;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::proto::sync_message::{Content as SyncContent, Sent};
use presage::proto::{EditMessage, SyncMessage};
use presage::store::{ContentExt, Store, Thread};
use presage_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

/// What Merel hears.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    /// The link to show as a QR code, for the Signal app on the phone to scan.
    Link {
        url: String,
    },
    Linked {
        number: Option<String>,
    },
    /// A message, sent or received; `thread` is the chat it belongs to.
    Message {
        thread: String,
        thread_name: String,
        group: bool,
        sender: String,
        sender_name: String,
        mine: bool,
        /// Milliseconds since the epoch, as Signal counts them; unique within a thread.
        timestamp: u64,
        text: String,
        attachments: Vec<String>,
    },
    /// Every message waiting on the server is in.
    Synced,
    Sent {
        thread: String,
        timestamp: u64,
    },
    Error {
        message: String,
    },
}

/// What Merel asks.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Send { thread: String, text: String },
}

fn emit(event: &Event) {
    if let Ok(line) = serde_json::to_string(event) {
        println!("{line}");
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(e) = start().await {
        emit(&Event::Error { message: format!("{e:#}") });
        std::process::exit(1);
    }
}

async fn start() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let key = std::env::var("MEREL_SIGNAL_KEY").context("MEREL_SIGNAL_KEY is not set")?;
    match args.get(1..).unwrap_or_default() {
        [cmd, dir, name] if cmd == "link" => link(open(dir, &key).await?, name).await,
        [cmd, dir] if cmd == "run" => run(Manager::load_registered(open(dir, &key).await?).await?).await,
        _ => bail!("usage: merel-signal link <dir> <device name> | merel-signal run <dir>"),
    }
}

async fn open(dir: &str, key: &str) -> Result<SqliteStore> {
    std::fs::create_dir_all(dir)?;
    let url = format!("sqlite://{dir}/signal.db?mode=rwc");
    // A new key of someone already known is trusted, as Signal itself does; Merel shows the
    // safety number for those who want to check.
    Ok(SqliteStore::open_with_passphrase(&url, Some(key), OnNewIdentity::Trust).await?)
}

async fn link(store: SqliteStore, name: &str) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    let (linked, _) = future::join(Manager::link_secondary_device(store, SignalServers::Production, name.to_string(), tx), async {
        if let Ok(url) = rx.await {
            emit(&Event::Link { url: url.to_string() });
        }
    })
    .await;
    let manager = linked.map_err(|e| anyhow!("{e}"))?;
    let number = manager.registration_data().phone_number.to_string();
    emit(&Event::Linked { number: Some(number) });
    Ok(())
}

async fn run<S: Store>(mut manager: Manager<S, Registered>) -> Result<()> {
    let sender = manager.clone();
    let messages = manager.receive_messages().await.map_err(|e| anyhow!("{e}"))?;
    pin_mut!(messages);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut sending = sender;
    loop {
        tokio::select! {
            received = messages.next() => match received {
                Some(Received::QueueEmpty) => emit(&Event::Synced),
                Some(Received::Content(content)) => {
                    if let Some(event) = describe(&manager, &content).await {
                        emit(&event);
                    }
                }
                Some(_) => {}
                None => bail!("the connection with Signal closed"),
            },
            line = lines.next_line() => match line? {
                Some(line) => match serde_json::from_str::<Command>(&line) {
                    Ok(Command::Send { thread, text }) => emit(&match send(&mut sending, &thread, &text).await {
                        Ok(timestamp) => Event::Sent { thread, timestamp },
                        Err(e) => Event::Error { message: format!("{e:#}") },
                    }),
                    Err(e) => emit(&Event::Error { message: format!("unknown command: {e}") }),
                },
                // Merel closed: done.
                None => return Ok(()),
            },
        }
    }
}

/// The thread's id as Merel keeps it: the person's service id, or `group:` and the group's key.
fn thread_id(thread: &Thread) -> String {
    match thread {
        Thread::Contact(id) => id.service_id_string(),
        Thread::Group(key) => format!("group:{}", hex::encode(key)),
    }
}

async fn contact_name<S: Store>(manager: &Manager<S, Registered>, id: &ServiceId) -> String {
    manager
        .store()
        .contact_by_id(id)
        .await
        .ok()
        .flatten()
        .map(|c| if c.name.is_empty() { c.phone_number.map(|p| p.to_string()).unwrap_or_default() } else { c.name })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| id.service_id_string())
}

/// A message worth showing and searching: text sent or received, here or on another device.
async fn describe<S: Store>(manager: &Manager<S, Registered>, content: &Content) -> Option<Event> {
    let (data, mine) = match &content.body {
        ContentBody::DataMessage(d) => (d, false),
        ContentBody::EditMessage(EditMessage { data_message: Some(d), .. }) => (d, false),
        ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(Sent { message: Some(d), .. })), .. }) => (d, true),
        ContentBody::SynchronizeMessage(SyncMessage {
            content: Some(SyncContent::Sent(Sent { edit_message: Some(EditMessage { data_message: Some(d), .. }), .. })),
            ..
        }) => (d, true),
        _ => return None,
    };
    let DataMessage { body, attachments, reaction, .. } = data;
    if reaction.is_some() {
        return None;
    }
    let names: Vec<String> = attachments.iter().filter_map(|a| a.file_name.clone()).collect();
    let text = body.clone().unwrap_or_default();
    if text.is_empty() && attachments.is_empty() {
        return None;
    }
    let thread = Thread::try_from(content).ok()?;
    let (thread_name, group) = match &thread {
        Thread::Contact(id) => (contact_name(manager, id).await, false),
        Thread::Group(key) => {
            let title = manager.store().group(*key).await.ok().flatten().map(|g| g.title).unwrap_or_default();
            (title, true)
        }
    };
    let me = &content.metadata.sender;
    Some(Event::Message {
        thread: thread_id(&thread),
        thread_name,
        group,
        sender: me.service_id_string(),
        sender_name: contact_name(manager, me).await,
        mine,
        timestamp: content.timestamp(),
        text,
        attachments: names,
    })
}

async fn send<S: Store>(manager: &mut Manager<S, Registered>, thread: &str, text: &str) -> Result<u64> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let message = DataMessage { body: Some(text.to_string()), timestamp: Some(timestamp), ..Default::default() };
    match thread.strip_prefix("group:") {
        Some(key) => {
            let key = hex::decode(key)?;
            manager.send_message_to_group(&key, message, timestamp).await.map_err(|e| anyhow!("{e}"))?;
        }
        None => {
            let to = ServiceId::parse_from_service_id_string(thread).ok_or_else(|| anyhow!("unknown chat {thread}"))?;
            manager.send_message(to, message, timestamp).await.map_err(|e| anyhow!("{e}"))?;
        }
    }
    Ok(timestamp)
}
