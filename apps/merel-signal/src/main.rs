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
//!                                          ← {"type":"react","thread":…,"emoji":…,…}
//! ```
//!
//! The store in `<dir>` is encrypted with the key in the environment variable `MEREL_SIGNAL_KEY`,
//! which Merel keeps in the system keychain.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use futures::channel::oneshot;
use futures::{StreamExt, future, pin_mut};
use presage::Manager;
use presage::libsignal_service::configuration::SignalServers;
use presage::libsignal_service::content::{Content, ContentBody, DataMessage};
use presage::libsignal_service::protocol::ServiceId;
use presage::libsignal_service::sender::AttachmentSpec;
use presage::manager::Registered;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::proto::AttachmentPointer;
use presage::proto::data_message::{Quote, Reaction, quote};
use presage::proto::sync_message::{Content as SyncContent, Sent};
use presage::proto::{EditMessage, SyncMessage};
use presage::proto::{ReceiptMessage, receipt_message, typing_message};
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
        attachments: Vec<FileInfo>,
        /// The message this one answers, quoted.
        #[serde(skip_serializing_if = "Option::is_none")]
        quote: Option<Quoted>,
    },
    /// Someone reacted with an emoji to a message, or took a reaction back (`remove`). The
    /// message is the one `target_author` sent at `target_timestamp`.
    Reaction {
        thread: String,
        sender: String,
        sender_name: String,
        mine: bool,
        timestamp: u64,
        emoji: String,
        remove: bool,
        target_author: String,
        target_timestamp: u64,
    },
    /// The contents of a file Merel asked for, in base64; `data` is empty when it could not
    /// be fetched, with why in `error`.
    File {
        id: String,
        data: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// `sender` got (`delivered`) or read (`read`) the account's messages sent at `timestamps`.
    Receipt {
        sender: String,
        kind: &'static str,
        timestamps: Vec<u64>,
    },
    /// `sender` started or stopped typing in the chat between the two of them.
    Typing {
        sender: String,
        sender_name: String,
        typing: bool,
    },
    /// The account read these messages on another device: by author and timestamp.
    ReadElsewhere {
        read: Vec<(String, u64)>,
    },
    /// The account's Signal settings, from the phone: whether to send read receipts and show
    /// typing.
    Settings {
        read_receipts: Option<bool>,
        typing_indicators: Option<bool>,
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

/// A file sent with a message. `id` is how Merel asks for its contents.
#[derive(Serialize, Clone)]
struct FileInfo {
    id: String,
    name: String,
    mime: String,
    size: u64,
}

/// A file Merel sends with a message, its contents in base64.
#[derive(Deserialize)]
struct OutFile {
    name: String,
    mime: String,
    data: String,
}

/// Every file of the messages passed on, by id, to fetch when Merel asks.
type Files = HashMap<String, AttachmentPointer>;

/// Most bytes a file to fetch or send may have: Signal's own limit is 100 MB.
const MAX_FILE: usize = 100 * 1024 * 1024;

/// The message a message answers: who sent it when, and (the start of) its text.
#[derive(Serialize, Deserialize, Clone)]
struct Quoted {
    author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    author_name: String,
    timestamp: u64,
    #[serde(default)]
    text: String,
}

/// What Merel asks.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Send {
        thread: String,
        text: String,
        /// The message this one answers.
        #[serde(default)]
        quote: Option<Quoted>,
        #[serde(default)]
        files: Vec<OutFile>,
    },
    /// The contents of file `id`, as `{"type":"file"}`.
    Fetch { id: String },
    /// Tells `author` their messages sent at `timestamps` were read. Merel only asks when the
    /// account's Signal settings send read receipts.
    MarkRead { author: String, timestamps: Vec<u64> },
    /// Reacts to the message `target_author` sent at `target_timestamp`; `remove` takes the
    /// reaction back. Signal keeps one reaction per person per message.
    React { thread: String, emoji: String, remove: bool, target_author: String, target_timestamp: u64 },
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
            let mut files = Files::new();
            // Without the stored messages Merel still gets the new ones.
            match threads(dir, &key).await {
                Ok(threads) => {
                    if let Err(e) = replay(&manager, &threads, &mut files).await {
                        eprintln!("merel-signal: the stored messages did not come: {e:#}");
                    }
                }
                Err(e) => eprintln!("merel-signal: the stored chats could not be listed: {e:#}"),
            }
            run(manager, dir, &key, files).await
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
async fn replay<S: Store>(manager: &Manager<S, Registered>, threads: &[Thread], files: &mut Files) -> Result<()> {
    let mut sent = 0;
    for thread in threads {
        let Ok(messages) = manager.store().messages(thread, ..).await else { continue };
        for content in messages.flatten() {
            if sent >= MAX_REPLAY {
                return Ok(());
            }
            if let Some(event) = describe(manager, &content, files).await {
                emit(&event);
                sent += 1;
            }
        }
    }
    Ok(())
}

async fn run<S: Store>(mut manager: Manager<S, Registered>, dir: &str, key: &str, mut files: Files) -> Result<()> {
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
                    if let Some(event) = status(&manager, &content).await {
                        emit(&event);
                        continue;
                    }
                    // What kind of message came, never who or what: for finding out why one
                    // does not show.
                    let shown = describe(&manager, &content, &mut files).await;
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
                    Ok(Command::Send { thread, text, quote, files: out }) => match send(&mut sending, dir, key, &thread, &text, quote.as_ref(), out).await {
                        Ok((to, timestamp, sent_files)) => {
                            let attachments = sent_files.into_iter().map(|p| info(&p, &mut files)).collect();
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
                                attachments,
                                quote,
                            });
                            emit(&Event::Sent { thread, timestamp });
                        }
                        Err(e) => emit(&Event::Error { message: format!("{e:#}") }),
                    },
                    Ok(Command::MarkRead { author, timestamps }) => {
                        let receipt = ReceiptMessage { r#type: Some(receipt_message::Type::Read as i32), timestamp: timestamps };
                        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
                        match ServiceId::parse_from_service_id_string(&author) {
                            Some(to) => {
                                if let Err(e) = sending.send_message(to, ContentBody::ReceiptMessage(receipt), now).await {
                                    eprintln!("merel-signal: a read receipt did not go: {e}");
                                }
                            }
                            None => eprintln!("merel-signal: a read receipt for no one"),
                        }
                    }
                    Ok(Command::Fetch { id }) => {
                        let event = match files.get(&id) {
                            Some(pointer) if pointer.size.unwrap_or(0) as usize <= MAX_FILE => match manager.get_attachment(pointer).await {
                                Ok(bytes) => Event::File { id, data: base64::engine::general_purpose::STANDARD.encode(bytes), error: None },
                                Err(e) => Event::File { id, data: String::new(), error: Some(format!("{e}")) },
                            },
                            Some(_) => Event::File { id, data: String::new(), error: Some("too big".into()) },
                            None => Event::File { id, data: String::new(), error: Some("unknown file".into()) },
                        };
                        emit(&event);
                    }
                    Ok(Command::React { thread, emoji, remove, target_author, target_timestamp }) => {
                        let reaction = Reaction {
                            emoji: Some(emoji.clone()),
                            remove: Some(remove),
                            target_author_aci: Some(target_author.clone()),
                            target_sent_timestamp: Some(target_timestamp),
                            ..Default::default()
                        };
                        let message = DataMessage { reaction: Some(reaction), ..Default::default() };
                        match send_data(&mut sending, dir, key, &thread, message).await {
                            Ok((_, timestamp)) => {
                                let me = ServiceId::from(manager.registration_data().service_ids.aci());
                                emit(&Event::Reaction {
                                    thread: thread.clone(),
                                    sender: me.service_id_string(),
                                    sender_name: contact_name(&manager, &me).await,
                                    mine: true,
                                    timestamp,
                                    emoji,
                                    remove,
                                    target_author,
                                    target_timestamp,
                                });
                                emit(&Event::Sent { thread, timestamp });
                            }
                            Err(e) => emit(&Event::Error { message: format!("{e:#}") }),
                        }
                    }
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

/// What a receipt, typing or a sync of reading or settings says; `None` for anything else.
async fn status<S: Store>(manager: &Manager<S, Registered>, content: &Content) -> Option<Event> {
    let sender = &content.metadata.sender;
    match &content.body {
        ContentBody::ReceiptMessage(r) => {
            let kind = match r.r#type.and_then(|t| receipt_message::Type::try_from(t).ok()) {
                Some(receipt_message::Type::Delivery) => "delivered",
                Some(receipt_message::Type::Read | receipt_message::Type::Viewed) => "read",
                None => return None,
            };
            Some(Event::Receipt { sender: sender.service_id_string(), kind, timestamps: r.timestamp.clone() })
        }
        // Typing in a group names the group only by an id Merel does not keep; between two
        // people the sender is the chat.
        ContentBody::TypingMessage(t) if t.group_id.is_none() => Some(Event::Typing {
            sender: sender.service_id_string(),
            sender_name: contact_name(manager, sender).await,
            typing: t.action.and_then(|a| typing_message::Action::try_from(a).ok()) == Some(typing_message::Action::Started),
        }),
        ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Configuration(c)), .. }) => {
            Some(Event::Settings { read_receipts: c.read_receipts, typing_indicators: c.typing_indicators })
        }
        ContentBody::SynchronizeMessage(m) if !m.read.is_empty() => {
            Some(Event::ReadElsewhere { read: m.read.iter().filter_map(|r| Some((r.sender_aci.clone()?, r.timestamp?))).collect() })
        }
        _ => None,
    }
}

/// The kind of a message, for the log.
fn kind(body: &ContentBody) -> &'static str {
    match body {
        ContentBody::DataMessage(DataMessage { reaction: Some(_), .. }) => "a reaction",
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
async fn describe<S: Store>(manager: &Manager<S, Registered>, content: &Content, files: &mut Files) -> Option<Event> {
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
    let DataMessage { body, attachments, reaction, quote, .. } = data;
    if let Some(r) = reaction {
        let thread = Thread::try_from(content).ok()?;
        let sender = &content.metadata.sender;
        let own = ServiceId::from(manager.registration_data().service_ids.aci());
        return Some(Event::Reaction {
            thread: thread_id(&thread),
            sender: sender.service_id_string(),
            sender_name: contact_name(manager, sender).await,
            mine: mine || *sender == own,
            timestamp: content.timestamp(),
            emoji: r.emoji.clone().filter(|e| !e.is_empty())?,
            remove: r.remove.unwrap_or(false),
            target_author: r.target_author_aci.clone()?,
            target_timestamp: r.target_sent_timestamp?,
        });
    }
    let names: Vec<FileInfo> = attachments.iter().map(|a| info(a, files)).collect();
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
        quote: match quote {
            Some(q) => quoted(manager, q).await,
            None => None,
        },
    })
}

async fn send<S: Store>(
    manager: &mut Manager<S, Registered>,
    dir: &str,
    key: &str,
    thread: &str,
    text: &str,
    quote: Option<&Quoted>,
    out: Vec<OutFile>,
) -> Result<(Thread, u64, Vec<AttachmentPointer>)> {
    let quote = quote.map(|q| Quote {
        id: Some(q.timestamp),
        author_aci: Some(q.author.clone()),
        text: Some(q.text.clone()).filter(|t| !t.is_empty()),
        r#type: Some(quote::Type::Normal as i32),
        ..Default::default()
    });
    let mut specs = Vec::new();
    for f in out {
        let bytes = base64::engine::general_purpose::STANDARD.decode(f.data.as_bytes())?;
        if bytes.len() > MAX_FILE {
            bail!("{} is too big for Signal", f.name);
        }
        let spec = AttachmentSpec {
            content_type: f.mime,
            length: bytes.len(),
            file_name: Some(f.name),
            preview: None,
            voice_note: None,
            borderless: None,
            width: None,
            height: None,
            caption: None,
            blur_hash: None,
        };
        specs.push((spec, bytes));
    }
    let mut pointers = Vec::new();
    for uploaded in manager.upload_attachments(specs).await.map_err(|e| anyhow!("{e}"))? {
        pointers.push(uploaded.map_err(|e| anyhow!("{e:?}"))?);
    }
    let body = Some(text.to_string()).filter(|t| !t.is_empty());
    let message = DataMessage { body, quote, attachments: pointers.clone(), ..Default::default() };
    let (to, timestamp) = send_data(manager, dir, key, thread, message).await?;
    Ok((to, timestamp, pointers))
}

/// What Merel hears of file `a`, kept under its id to fetch later.
fn info(a: &AttachmentPointer, files: &mut Files) -> FileInfo {
    use sha2::Digest as _;
    let seed = a.digest.clone().or_else(|| a.key.clone()).unwrap_or_default();
    let id = hex::encode(&sha2::Sha256::digest(seed)[..12]);
    files.insert(id.clone(), a.clone());
    FileInfo {
        id,
        name: a.file_name.clone().unwrap_or_default(),
        mime: a.content_type.clone().unwrap_or_else(|| "application/octet-stream".into()),
        size: u64::from(a.size.unwrap_or(0)),
    }
}

/// The message `q` quotes, with its author's name.
async fn quoted<S: Store>(manager: &Manager<S, Registered>, q: &Quote) -> Option<Quoted> {
    let author = q.author_aci.clone()?;
    let name = match ServiceId::parse_from_service_id_string(&author) {
        Some(id) => contact_name(manager, &id).await,
        None => String::new(),
    };
    let mut text = q.text.clone().unwrap_or_default();
    if text.is_empty() {
        text = q.attachments.iter().filter_map(|a| a.file_name.clone()).collect::<Vec<_>>().join(", ");
    }
    Some(Quoted { author, author_name: name, timestamp: q.id?, text })
}

/// Sends `message` to the chat `thread`, stamped with the time now.
async fn send_data<S: Store>(
    manager: &mut Manager<S, Registered>,
    dir: &str,
    key: &str,
    thread: &str,
    mut message: DataMessage,
) -> Result<(Thread, u64)> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    message.timestamp = Some(timestamp);
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
