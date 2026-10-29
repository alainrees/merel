//! `merel-signal`: Merel's link to Signal, as a separate program.
//!
//! It links as a secondary device of the person's Signal account (as Signal Desktop does) and
//! speaks JSON lines: one event per line on stdout, one command per line on stdin.
//!
//! ```text
//! merel-signal link <dir> <device name>   → {"type":"link","url":…} then {"type":"linked"}
//! merel-signal run <dir>                   → {"type":"message",…}… and {"type":"synced"}
//!                                            (first every stored message, then the new ones)
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
    /// `id` is the account's own service id: the `sender` of what it sends.
    Linked {
        number: Option<String>,
        id: String,
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
        [cmd, dir] if cmd == "run" => {
            let manager = Manager::load_registered(open(dir, &key).await?).await?;
            // Without the stored messages Merel still gets the new ones.
            match threads(dir, &key).await {
                Ok(threads) => {
                    if let Err(e) = replay(&manager, &threads).await {
                        eprintln!("merel-signal: the stored messages did not come: {e:#}");
                    }
                }
                Err(e) => eprintln!("merel-signal: the stored chats could not be listed: {e:#}"),
            }
            run(manager, dir, &key).await
        }
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
    let data = manager.registration_data();
    let id = ServiceId::from(data.service_ids.aci()).service_id_string();
    emit(&Event::Linked { number: Some(data.phone_number.to_string()), id });
    Ok(())
}

/// Most stored messages passed on at the start, the newest of each chat first.
const MAX_REPLAY: usize = 20_000;

/// Passes on every message the store keeps, so Merel has the whole conversation each time it
/// starts the helper (and keeps nothing of its own).
async fn replay<S: Store>(manager: &Manager<S, Registered>, threads: &[Thread]) -> Result<()> {
    let mut sent = 0;
    for thread in threads {
        let Ok(messages) = manager.store().messages(thread, ..).await else { continue };
        for content in messages.flatten() {
            if sent >= MAX_REPLAY {
                return Ok(());
            }
            if let Some(event) = describe(manager, &content).await {
                emit(&event);
                sent += 1;
            }
        }
    }
    Ok(())
}

async fn run<S: Store>(mut manager: Manager<S, Registered>, dir: &str, key: &str) -> Result<()> {
    let sender = manager.clone();
    let messages = manager.receive_messages().await.map_err(|e| anyhow!("{e}"))?;
    pin_mut!(messages);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut sending = sender;
    loop {
        tokio::select! {
            received = messages.next() => match received {
                Some(Received::QueueEmpty) => {
                    eprintln!("merel-signal: all waiting messages are in");
                    emit(&Event::Synced)
                }
                Some(Received::Content(content)) => {
                    // What kind of message came, never who or what: for finding out why one
                    // does not show.
                    let shown = describe(&manager, &content).await;
                    eprintln!("merel-signal: received {}, {}", kind(&content.body), if shown.is_some() { "shown" } else { "not shown" });
                    if let Some(event) = shown {
                        emit(&event);
                    }
                }
                Some(Received::Contacts) => eprintln!("merel-signal: contacts synced"),
                Some(Received::DecryptionError(_)) => eprintln!("merel-signal: a message could not be decrypted"),
                None => bail!("the connection with Signal closed"),
            },
            line = lines.next_line() => match line? {
                Some(line) => match serde_json::from_str::<Command>(&line) {
                    Ok(Command::Send { thread, text }) => match send(&mut sending, dir, key, &thread, &text).await {
                        Ok((to, timestamp)) => {
                            // As it came from another device of the account's, so Merel lists it.
                            let me = ServiceId::from(manager.registration_data().service_ids.aci());
                            let (thread_name, group) = name_of(&manager, &to).await;
                            emit(&Event::Message {
                                thread: thread.clone(),
                                thread_name,
                                group,
                                sender: me.service_id_string(),
                                sender_name: contact_name(&manager, &me).await,
                                mine: true,
                                timestamp,
                                text,
                                attachments: Vec::new(),
                            });
                            emit(&Event::Sent { thread, timestamp });
                        }
                        Err(e) => emit(&Event::Error { message: format!("{e:#}") }),
                    },
                    Err(e) => emit(&Event::Error { message: format!("unknown command: {e}") }),
                },
                // Merel closed: done.
                None => return Ok(()),
            },
        }
    }
}

/// The thread's id as Merel keeps it: the person's service id, or `g` and a hash of the group's
/// key. The key itself lets anyone into the group, so it never leaves the helper.
fn thread_id(thread: &Thread) -> String {
    match thread {
        Thread::Contact(id) => id.service_id_string(),
        Thread::Group(key) => group_id(key),
    }
}

fn group_id(key: &[u8; 32]) -> String {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest([b"merel-signal-group:".as_slice(), key].concat());
    format!("g{}", hex::encode(&hash[..16]))
}

/// Every chat in the store. The store has no list of them, so it comes from its table.
async fn threads(dir: &str, key: &str) -> Result<Vec<Thread>> {
    use sqlx::{ConnectOptions as _, Connection as _, Row as _};
    let options: sqlx::sqlite::SqliteConnectOptions = format!("sqlite://{dir}/signal.db?mode=ro").parse()?;
    let mut db = options.pragma("key", format!("'{}'", key.replace('\'', "''"))).connect().await?;
    let rows = sqlx::query("SELECT group_master_key, recipient_id FROM threads").fetch_all(&mut db).await?;
    db.close().await?;
    let mut threads = Vec::new();
    for row in rows {
        let group: Option<Vec<u8>> = row.try_get(0)?;
        let person: Option<uuid::Uuid> = row.try_get(1)?;
        match (group.and_then(|g| <[u8; 32]>::try_from(g).ok()), person) {
            (Some(key), _) => threads.push(Thread::Group(key)),
            (None, Some(id)) => threads.push(Thread::Contact(ServiceId::Aci(id.into()))),
            (None, None) => {}
        }
    }
    Ok(threads)
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

/// The kind of a message, for the log.
fn kind(body: &ContentBody) -> &'static str {
    match body {
        ContentBody::DataMessage(_) => "a message",
        ContentBody::EditMessage(_) => "an edit",
        ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(_)), .. }) => "a message sent from another device",
        ContentBody::SynchronizeMessage(_) => "a sync message",
        ContentBody::CallMessage(_) => "a call",
        ContentBody::ReceiptMessage(_) => "a receipt",
        ContentBody::TypingMessage(_) => "typing",
        _ => "something else",
    }
}

/// The chat's name, and whether it is a group.
async fn name_of<S: Store>(manager: &Manager<S, Registered>, thread: &Thread) -> (String, bool) {
    match thread {
        Thread::Contact(id) => (contact_name(manager, id).await, false),
        Thread::Group(key) => (manager.store().group(*key).await.ok().flatten().map(|g| g.title).unwrap_or_default(), true),
    }
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
    let (thread_name, group) = name_of(manager, &thread).await;
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

async fn send<S: Store>(manager: &mut Manager<S, Registered>, dir: &str, key: &str, thread: &str, text: &str) -> Result<(Thread, u64)> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let message = DataMessage { body: Some(text.to_string()), timestamp: Some(timestamp), ..Default::default() };
    match thread.starts_with('g') {
        true => {
            let groups = threads(dir, key).await?;
            let Some(Thread::Group(key)) = groups.iter().find(|t| matches!(t, Thread::Group(k) if group_id(k) == thread)) else {
                bail!("unknown chat {thread}");
            };
            manager.send_message_to_group(key, message, timestamp).await.map_err(|e| anyhow!("{e}"))?;
            Ok((Thread::Group(*key), timestamp))
        }
        false => {
            let to = ServiceId::parse_from_service_id_string(thread).ok_or_else(|| anyhow!("unknown chat {thread}"))?;
            manager.send_message(to, message, timestamp).await.map_err(|e| anyhow!("{e}"))?;
            Ok((Thread::Contact(to), timestamp))
        }
    }
}
