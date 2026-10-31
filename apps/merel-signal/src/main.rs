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
//!                                          ← {"type":"edit","thread":…,"timestamp":…,"text":…}
//!                                          ← {"type":"delete","thread":…,"timestamp":…,"everyone":…}
//! ```
//!
//! The store in `<dir>` is encrypted with the key in the environment variable `MEREL_SIGNAL_KEY`,
//! which Merel keeps in the system keychain.

mod phone;
mod wire;

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use futures::channel::oneshot;
use futures::{StreamExt, future, pin_mut};
use presage::Manager;
use presage::libsignal_service::configuration::SignalServers;
use presage::libsignal_service::content::Metadata;
use presage::libsignal_service::content::{Content, ContentBody, DataMessage};
use presage::libsignal_service::prelude::{GroupMasterKey, GroupSecretParams};
use presage::libsignal_service::protocol::ServiceId;
use presage::libsignal_service::sender::AttachmentSpec;
use presage::manager::Registered;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::proto::AttachmentPointer;
use presage::proto::data_message::{Delete, Quote, Reaction, quote};
use presage::proto::sync_message::{CallEvent, Content as SyncContent, DeleteForMe, Sent, call_event, delete_for_me};
use presage::proto::{AddressableMessage, ConversationIdentifier, addressable_message, conversation_identifier};
use presage::proto::{CallMessage, EditMessage, GroupContextV2, SyncMessage, call_message};
use presage::proto::{ReceiptMessage, receipt_message, typing_message};
use presage::store::{ContentExt, Store, Thread};
use presage_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::phone::Phone;
use crate::wire::Phase;

/// What Merel hears.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    /// The link to show as a QR code, for the Signal app on the phone to scan.
    /// The chat `thread` was deleted on another device of the account's, so it goes here too.
    ChatDeleted {
        thread: String,
    },
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
        /// Set when this is a call rather than a message: the text is then empty.
        #[serde(skip_serializing_if = "Option::is_none")]
        call: Option<Call>,
        /// When the writer last changed the text after sending it; 0 when never.
        #[serde(skip_serializing_if = "is_zero")]
        edited: u64,
        /// The writer deleted it for everyone: only that it was there is left.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        deleted: bool,
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
    /// The account's Signal contacts, as the phone shares them: to start a chat with.
    Contacts {
        people: Vec<Person>,
    },
    /// Which of the numbers Merel asked about are on Signal, with their service id; the
    /// numbers that are not are left out.
    Found {
        people: Vec<Person>,
    },
    /// Every message waiting on the server is in. `calls` says whether this build can call.
    Synced {
        calls: bool,
    },
    /// Whether calls can be made from here now (the calls program runs and has sound).
    Calls {
        available: bool,
    },
    /// How a call in Merel goes: ringing, connected, ended….
    Call {
        thread: String,
        name: String,
        call_id: u64,
        state: Phase,
    },
    Sent {
        thread: String,
        timestamp: u64,
    },
    Error {
        message: String,
    },
}

/// A Signal contact: `id` is their service id, the `thread` of a chat with them.
#[derive(Serialize)]
struct Person {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    number: String,
}

/// The contacts the phone shared, by name; ones without a name or number are left out, as
/// Signal leaves them out of its own list.
async fn contacts<S: Store>(manager: &Manager<S, Registered>) -> Vec<Person> {
    let Ok(all) = manager.store().contacts().await else { return Vec::new() };
    let mut people: Vec<Person> = all
        .flatten()
        .filter_map(|c| {
            let number = c.phone_number.map(|p| p.to_string()).unwrap_or_default();
            let name = if c.name.trim().is_empty() { number.clone() } else { c.name.trim().to_string() };
            (!name.is_empty()).then(|| Person { id: ServiceId::Aci(c.uuid.into()).service_id_string(), name, number })
        })
        .collect();
    people.sort_by_key(|p| p.name.to_lowercase());
    people
}

/// A file sent with a message. `id` is how Merel asks for its contents.
#[derive(Serialize, Clone)]
struct FileInfo {
    id: String,
    name: String,
    mime: String,
    size: u64,
    /// A voice message, recorded in the app, not a file sent.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    voice: bool,
}

/// A file Merel sends with a message, its contents in base64.
#[derive(Deserialize)]
struct OutFile {
    name: String,
    mime: String,
    data: String,
    /// A voice message: Signal shows it with a play button.
    #[serde(default)]
    voice: bool,
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

/// A Signal call, as Merel shows it in the chat. Merel cannot take part in calls (yet): it
/// shows that someone is calling, and how the call ended.
#[derive(Serialize, Clone, Copy)]
struct Call {
    /// Signal's id of the call; later news about the same call has the same id.
    id: u64,
    kind: CallKind,
    state: CallState,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CallKind {
    Audio,
    Video,
    Group,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CallState {
    /// Ringing now: on the phone, as Merel cannot answer.
    Ringing,
    /// Answered, here or on another device; or for a call the account made, it went through.
    Answered,
    /// Nobody answered, or it was declined.
    Missed,
    /// A group call started.
    Started,
}

/// What Merel asks.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    /// Asks Signal which of these phone numbers (as +31612345678) are on Signal, as Signal
    /// itself does with the address book: through its contact discovery service, where the
    /// numbers are looked up inside an enclave nobody can look into.
    Discover {
        numbers: Vec<String>,
    },
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
    Fetch {
        id: String,
    },
    /// Tells `author` their messages sent at `timestamps` were read. Merel only asks when the
    /// account's Signal settings send read receipts.
    MarkRead {
        author: String,
        timestamps: Vec<u64>,
    },
    /// Reacts to the message `target_author` sent at `target_timestamp`; `remove` takes the
    /// reaction back. Signal keeps one reaction per person per message.
    React {
        thread: String,
        emoji: String,
        remove: bool,
        target_author: String,
        target_timestamp: u64,
    },
    /// Call the person of this chat (sound only).
    Call {
        thread: String,
    },
    /// Answer the call ringing.
    Accept {
        call_id: u64,
    },
    /// End the call, or decline the one ringing.
    HangUp,
    Mute {
        on: bool,
    },
    /// Replaces the text of the account's own message sent at `timestamp`, as Signal does.
    Edit {
        thread: String,
        timestamp: u64,
        text: String,
    },
    /// Deletes the account's message sent at `timestamp`: for `everyone`, as Signal does, else
    /// only here.
    Delete {
        thread: String,
        timestamp: u64,
        everyone: bool,
    },
    /// Deletes the whole chat here and on the account's other devices, as Signal's "Delete
    /// chat" does; the others in it keep their messages.
    DeleteChat {
        thread: String,
    },
}

fn is_zero(n: &u64) -> bool {
    *n == 0
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
                    let at = std::time::Instant::now();
                    if let Err(e) = replay(&manager, &threads, &mut files).await {
                        eprintln!("merel-signal: the stored messages did not come: {e:#}");
                    }
                    eprintln!("merel-signal: {} stored chats passed on in {} ms", threads.len(), at.elapsed().as_millis());
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

/// Asks merel-signal-lookup, next to this program, which of `numbers` are on Signal.
async fn look_up(registration: serde_json::Value, numbers: Vec<String>) -> std::result::Result<Vec<Person>, String> {
    use tokio::io::AsyncWriteExt as _;
    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Answer {
        Found { people: Vec<Found> },
        Error { message: String },
    }
    #[derive(Deserialize)]
    struct Found {
        id: String,
        number: String,
    }
    let program = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name(format!("merel-signal-lookup{}", std::env::consts::EXE_SUFFIX));
    let mut child = tokio::process::Command::new(&program)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let asked = serde_json::json!({ "registration": registration, "numbers": numbers });
    let mut stdin = child.stdin.take().ok_or("no input")?;
    stdin.write_all(format!("{asked}\n").as_bytes()).await.map_err(|e| e.to_string())?;
    drop(stdin);
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    let line = String::from_utf8_lossy(&out.stdout);
    match serde_json::from_str::<Answer>(line.trim()).map_err(|e| e.to_string())? {
        Answer::Found { people } => Ok(people.into_iter().map(|f| Person { id: f.id, name: String::new(), number: f.number }).collect()),
        Answer::Error { message } => Err(message),
    }
}

/// How many seconds ago `sent` (milliseconds since 1970) was: how long a message took to come.
fn late(sent: u64) -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    u64::try_from(now.saturating_sub(u128::from(sent)) / 1000).unwrap_or(u64::MAX)
}

/// Most calls kept ringing at once; a caller that never hangs up is forgotten.
const MAX_RINGING: usize = 64;

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
    let started = std::time::Instant::now();
    let sender = manager.clone();
    let messages = manager.receive_messages().await.map_err(|e| anyhow!("{e}"))?;
    eprintln!("merel-signal: connected to Signal in {} ms", started.elapsed().as_millis());
    pin_mut!(messages);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut sending = sender;
    let mut ringing = Ringing::default();
    let (mut phone, mut notes) = Phone::start();
    let mut calls_open = true;
    // Signal on the phone knows which of its contacts are on Signal; a linked device asks it
    // for that list, as Signal Desktop does, once the waiting messages are in.
    let mut contacts_asked = false;
    let mut asking: Option<future::LocalBoxFuture<'static, std::result::Result<(), String>>> = None;
    // A lookup of address book numbers, also on the side: it goes to another service.
    let mut finding: Option<future::LocalBoxFuture<'static, std::result::Result<Vec<Person>, String>>> = None;
    loop {
        tokio::select! {
            found = async {
                match finding.as_mut() {
                    Some(f) => f.await,
                    None => Ok(Vec::new()),
                }
            }, if finding.is_some() => {
                finding = None;
                match found {
                    Ok(people) => {
                        eprintln!("merel-signal: {} of the numbers asked about are on Signal", people.len());
                        emit(&Event::Found { people });
                    }
                    Err(e) => eprintln!("merel-signal: could not look up the address book: {e}"),
                }
            }
            asked = async {
                match asking.as_mut() {
                    Some(f) => f.await,
                    None => Ok(()),
                }
            }, if asking.is_some() => {
                asking = None;
                if let Err(e) = asked {
                    eprintln!("merel-signal: could not ask the phone for its contacts: {e}");
                }
            }
            note = notes.recv(), if calls_open => match note {
                Some(note) => {
                    for event in phone.handle(note, &mut sending, &mut ringing).await {
                        emit(&event);
                    }
                }
                None => {
                    calls_open = false;
                    if let Some(event) = phone.stopped() {
                        emit(&event);
                    }
                }
            },
            received = messages.next() => match received {
                Some(Received::QueueEmpty) => {
                    eprintln!("merel-signal: all waiting messages are in, {} s after the start", started.elapsed().as_secs());
                    emit(&Event::Contacts { people: contacts(&manager).await });
                    emit(&Event::Synced { calls: phone.ready() });
                    // Asked on the side: sending it can take long, and Merel waits for
                    // `synced` before it shows the chats.
                    if !contacts_asked {
                        contacts_asked = true;
                        let mut asker = sending.clone();
                        asking = Some(Box::pin(async move { asker.request_contacts().await.map_err(|e| e.to_string()) }));
                    }
                }
                Some(Received::Content(content)) => {
                    if let ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::DeleteForMe(d)), .. }) = &content.body {
                        for thread in chats_deleted(&manager, d).await {
                            emit(&Event::ChatDeleted { thread });
                        }
                        continue;
                    }
                    if let Some(event) = status(&manager, &content).await {
                        emit(&event);
                        continue;
                    }
                    // What kind of message came, never who or what: for finding out why one
                    // does not show.
                    let shown = match &content.body {
                        ContentBody::CallMessage(call) => {
                            phone.received(&manager, &content, call).await;
                            ringing.hear(&manager, &content, call).await
                        }
                        _ => describe(&manager, &content, &mut files).await,
                    };
                    eprintln!(
                        "merel-signal: received {}, {}, {} s after it was sent",
                        kind(&content.body),
                        if shown.is_some() { "shown" } else { "not shown" },
                        late(content.timestamp())
                    );
                    if let Some(event) = shown {
                        emit(&event);
                    }
                }
                Some(Received::Contacts) => {
                    eprintln!("merel-signal: contacts synced");
                    emit(&Event::Contacts { people: contacts(&manager).await });
                }
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
                                call: None,
                                edited: 0,
                                deleted: false,
                            });
                            emit(&Event::Sent { thread, timestamp });
                        }
                        Err(e) => emit(&Event::Error { message: format!("{e:#}") }),
                    },
                    Ok(Command::Edit { thread, timestamp, text }) => match edit(&mut sending, dir, key, &thread, timestamp, &text).await {
                        Ok(edited) => {
                            if let Some(event) = describe(&manager, &edited, &mut files).await {
                                emit(&event);
                            }
                            emit(&Event::Sent { thread, timestamp });
                        }
                        Err(e) => emit(&Event::Error { message: format!("{e:#}") }),
                    },
                    Ok(Command::Delete { thread, timestamp, everyone }) => match delete(&mut sending, dir, key, &thread, timestamp, everyone).await {
                        Ok(left) => {
                            if let Some(event) = left {
                                emit(&event);
                            }
                            emit(&Event::Sent { thread, timestamp });
                        }
                        Err(e) => emit(&Event::Error { message: format!("{e:#}") }),
                    },
                    // Merel let go of the chat already; should telling the phone fail, it stays
                    // gone here all the same.
                    Ok(Command::Discover { numbers }) => {
                        if finding.is_none() {
                            let data = manager.registration_data();
                            let mut registration = serde_json::to_value(data).map_err(|e| anyhow!("{e}"))?;
                            // The number as text, +31612345678, rather than its parts.
                            registration["phone_number"] = data.phone_number.format().mode(presage::libsignal_service::prelude::phonenumber::Mode::E164).to_string().into();
                            finding = Some(Box::pin(look_up(registration, numbers)));
                        }
                    }
                    Ok(Command::DeleteChat { thread }) => {
                        if let Err(e) = delete_chat(&mut sending, dir, key, &thread).await {
                            eprintln!("merel-signal: deleting a chat: {e:#}");
                        }
                    }
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
                    Ok(Command::Call { thread }) => {
                        if let Err(e) = phone.call(&manager, &thread).await {
                            emit(&Event::Error { message: format!("{e:#}") });
                        }
                    }
                    Ok(Command::Accept { call_id }) => {
                        if let Err(e) = phone.accept(call_id).await {
                            emit(&Event::Error { message: format!("{e:#}") });
                        }
                    }
                    Ok(Command::HangUp) => {
                        if let Err(e) = phone.hang_up().await {
                            emit(&Event::Error { message: format!("{e:#}") });
                        }
                    }
                    Ok(Command::Mute { on }) => {
                        if let Err(e) = phone.mute(on).await {
                            emit(&Event::Error { message: format!("{e:#}") });
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
    if let ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::CallEvent(event)), .. }) = &content.body {
        return logged_call(manager, content, event).await;
    }
    let (data, mine, edited) = match &content.body {
        ContentBody::DataMessage(d) => (d, false, false),
        ContentBody::EditMessage(EditMessage { data_message: Some(d), .. }) => (d, false, true),
        ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(Sent { message: Some(d), .. })), .. }) => (d, true, false),
        ContentBody::SynchronizeMessage(SyncMessage {
            content: Some(SyncContent::Sent(Sent { edit_message: Some(EditMessage { data_message: Some(d), .. }), .. })),
            ..
        }) => (d, true, true),
        _ => return None,
    };
    // What Merel itself sent is kept as the message, not as a sync from another device: it is
    // the account's own, in the chat it went to.
    let own = ServiceId::from(manager.registration_data().service_ids.aci());
    let sent_here = !mine && content.metadata.sender == own;
    let mine = mine || sent_here;
    let thread = match &data.group_v2 {
        None if sent_here => Thread::Contact(content.metadata.destination),
        _ => Thread::try_from(content).ok()?,
    };
    let DataMessage { body, attachments, reaction, quote, group_call_update, delete, .. } = data;
    if let Some(Delete { target_sent_timestamp: Some(target) }) = delete {
        return deleted(manager, content, thread, *target).await;
    }
    if let Some(r) = reaction {
        let sender = &content.metadata.sender;
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
    if group_call_update.is_some() {
        let call = Call { id: content.timestamp(), kind: CallKind::Group, state: CallState::Started };
        return chat_call(manager, thread, &content.metadata.sender, mine, call, content.timestamp()).await;
    }
    let names: Vec<FileInfo> = attachments.iter().map(|a| info(a, files)).collect();
    let text = body.clone().unwrap_or_default();
    if text.is_empty() && attachments.is_empty() {
        return None;
    }
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
        call: None,
        edited: if edited { data.timestamp.unwrap_or(1) } else { 0 },
        deleted: false,
    })
}

/// A call as an entry in its chat, from `caller`.
async fn chat_call<S: Store>(
    manager: &Manager<S, Registered>,
    thread: Thread,
    caller: &ServiceId,
    mine: bool,
    call: Call,
    timestamp: u64,
) -> Option<Event> {
    let (thread_name, group) = name_of(manager, &thread).await;
    Some(Event::Message {
        thread: thread_id(&thread),
        thread_name,
        group,
        sender: caller.service_id_string(),
        sender_name: contact_name(manager, caller).await,
        mine,
        timestamp,
        text: String::new(),
        attachments: Vec::new(),
        quote: None,
        call: Some(call),
        edited: 0,
        deleted: false,
    })
}

/// A finished call between two people, as the account's devices log it.
async fn logged_call<S: Store>(manager: &Manager<S, Registered>, content: &Content, event: &CallEvent) -> Option<Event> {
    use call_event::{Direction, Event as Outcome, Type};
    let kind = match event.r#type() {
        Type::AudioCall => CallKind::Audio,
        Type::VideoCall => CallKind::Video,
        // Group calls show as the group's "call started" message instead.
        Type::GroupCall | Type::AdHocCall | Type::UnknownType => return None,
    };
    let state = match event.event() {
        Outcome::Accepted => CallState::Answered,
        Outcome::NotAccepted => CallState::Missed,
        Outcome::Observed | Outcome::Delete | Outcome::UnknownEvent => return None,
    };
    let peer = ServiceId::parse_from_service_id_binary(event.conversation_id.as_deref()?)?;
    let mine = event.direction() == Direction::Outgoing;
    let me = ServiceId::from(manager.registration_data().service_ids.aci());
    let caller = if mine { me } else { peer };
    // An outgoing call that nobody answered is not "missed" by the account itself.
    let state = if mine && state == CallState::Missed { CallState::Answered } else { state };
    let call = Call { id: event.call_id?, kind, state };
    let started = event.timestamp.unwrap_or_else(|| content.timestamp());
    chat_call(manager, Thread::Contact(peer), &caller, mine, call, started).await
}

/// Calls ringing now, by Signal's id of the call: who calls, what kind, and when it started.
#[derive(Default)]
struct Ringing(HashMap<u64, (ServiceId, CallKind, u64)>);

impl Ringing {
    /// Merel itself took the call: how it ends is Merel's to log.
    fn taken(&mut self, call_id: u64) {
        self.0.remove(&call_id);
    }
}

impl Ringing {
    /// What the signalling of a call says for Merel: someone calls, or how their call ended.
    /// An ended call goes into the store as the call log entry Signal keeps, so it is still
    /// there after a restart (signalling itself is not kept).
    async fn hear<S: Store>(&mut self, manager: &Manager<S, Registered>, content: &Content, message: &CallMessage) -> Option<Event> {
        use call_message::hangup::Type as Why;
        let me = ServiceId::from(manager.registration_data().service_ids.aci());
        if let Some(offer) = &message.offer {
            let caller = content.metadata.sender;
            if caller == me {
                return None;
            }
            let kind = match offer.r#type() {
                call_message::offer::Type::OfferVideoCall => CallKind::Video,
                call_message::offer::Type::OfferAudioCall => CallKind::Audio,
            };
            let id = offer.id?;
            if self.0.len() >= MAX_RINGING {
                self.0.clear();
            }
            self.0.insert(id, (caller, kind, content.timestamp()));
            let call = Call { id, kind, state: CallState::Ringing };
            return chat_call(manager, Thread::Contact(caller), &caller, false, call, content.timestamp()).await;
        }
        let (id, state) = if let Some(hangup) = &message.hangup {
            let state = match hangup.r#type() {
                Why::HangupAccepted => CallState::Answered,
                Why::HangupNormal | Why::HangupDeclined | Why::HangupBusy | Why::HangupNeedPermission => CallState::Missed,
            };
            (hangup.id?, state)
        } else if let Some(busy) = &message.busy {
            (busy.id?, CallState::Missed)
        } else {
            return None;
        };
        let (caller, kind, started) = self.0.remove(&id)?;
        let log = CallEvent {
            conversation_id: Some(caller.service_id_binary()),
            call_id: Some(id),
            timestamp: Some(started),
            r#type: Some(match kind {
                CallKind::Video => call_event::Type::VideoCall,
                _ => call_event::Type::AudioCall,
            } as i32),
            direction: Some(call_event::Direction::Incoming as i32),
            event: Some(match state {
                CallState::Answered => call_event::Event::Accepted,
                _ => call_event::Event::NotAccepted,
            } as i32),
        };
        let event = logged_call(manager, content, &log).await;
        let entry = Content {
            metadata: Metadata {
                sender: me,
                destination: me,
                sender_device: content.metadata.sender_device,
                server_guid: None,
                client_timestamp: content.metadata.client_timestamp,
                server_timestamp: content.metadata.server_timestamp,
                needs_receipt: false,
                unidentified_sender: false,
                was_plaintext: false,
                pni_verified: None,
            },
            body: ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::CallEvent(log)), ..Default::default() }),
        };

        if let Err(e) = manager.store().save_message(&Thread::Contact(caller), entry).await {
            emit(&Event::Error { message: format!("could not keep the call: {e}") });
        }
        event
    }
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
            voice_note: f.voice.then_some(true),
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
        voice: a.flags.unwrap_or(0) & VOICE_MESSAGE != 0,
    }
}

/// The flag Signal sets on an attachment that is a voice message.
const VOICE_MESSAGE: u32 = 1;

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
    let timestamp = now()?;
    message.timestamp = Some(timestamp);
    let to = find_thread(dir, key, thread).await?;
    in_group(manager, &to, &mut message).await;
    send_body(manager, &to, message.into(), timestamp).await?;
    Ok((to, timestamp))
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64)
}

/// The chat Merel names `thread`: a person's service id, or a group by its hashed key.
async fn find_thread(dir: &str, key: &str, thread: &str) -> Result<Thread> {
    if thread.starts_with('g') {
        let groups = threads(dir, key).await?;
        groups.into_iter().find(|t| matches!(t, Thread::Group(k) if group_id(k) == thread)).ok_or_else(|| anyhow!("unknown chat {thread}"))
    } else {
        ServiceId::parse_from_service_id_string(thread).map(Thread::Contact).ok_or_else(|| anyhow!("unknown chat {thread}"))
    }
}

/// For a group, says in `message` which group it is for, as Signal wants it: the members'
/// apps put it in the group by that.
async fn in_group<S: Store>(manager: &Manager<S, Registered>, thread: &Thread, message: &mut DataMessage) {
    if let Thread::Group(key) = thread
        && message.group_v2.is_none()
    {
        let revision = manager.store().group(*key).await.ok().flatten().map(|g| g.revision);
        message.group_v2 = Some(GroupContextV2 { master_key: Some(key.to_vec()), revision, ..Default::default() });
    }
}

async fn send_body<S: Store>(manager: &mut Manager<S, Registered>, to: &Thread, body: ContentBody, timestamp: u64) -> Result<()> {
    match to {
        Thread::Group(key) => manager.send_message_to_group(key, body, timestamp).await,
        Thread::Contact(id) => manager.send_message(*id, body, timestamp).await,
    }
    .map_err(|e| anyhow!("{e}"))
}

/// The message the account sent at `timestamp` in `to`, as it went out first.
async fn own_message<S: Store>(manager: &Manager<S, Registered>, to: &Thread, timestamp: u64) -> Result<(Content, DataMessage)> {
    let me = ServiceId::from(manager.registration_data().service_ids.aci());
    let original = manager.store().message(to, timestamp).await?.ok_or_else(|| anyhow!("unknown message"))?;
    if original.metadata.sender != me {
        bail!("only one's own message can be changed");
    }
    let data = match &original.body {
        ContentBody::DataMessage(d) => d.clone(),
        ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(Sent { message: Some(d), .. })), .. }) => d.clone(),
        _ => bail!("this message cannot be changed"),
    };
    Ok((original, data))
}

/// Replaces the text of the account's message sent at `timestamp`, keeping what else it had
/// (files, the message it answers), as Signal does; gives back the message as it is now.
async fn edit<S: Store>(manager: &mut Manager<S, Registered>, dir: &str, key: &str, thread: &str, timestamp: u64, text: &str) -> Result<Content> {
    let to = find_thread(dir, key, thread).await?;
    let (original, mut data) = own_message(manager, &to, timestamp).await?;
    if text.trim().is_empty() && data.attachments.is_empty() {
        bail!("an edit needs text");
    }
    let at = now()?;
    data.body = Some(text.to_string()).filter(|t| !t.is_empty());
    data.timestamp = Some(at);
    in_group(manager, &to, &mut data).await;
    let edit = EditMessage { target_sent_timestamp: Some(timestamp), data_message: Some(data) };
    send_body(manager, &to, edit.clone().into(), at).await?;
    // As another device of the account's would pass it on.
    let destination_service_id = match &to {
        Thread::Contact(id) => Some(id.service_id_string()),
        Thread::Group(_) => None,
    };
    let sent = Sent { destination_service_id, timestamp: Some(at), edit_message: Some(edit), ..Default::default() };
    Ok(Content {
        metadata: original.metadata,
        body: ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(sent)), ..Default::default() }),
    })
}

/// Deletes the account's message sent at `timestamp`: for everyone, as Signal does, or only
/// from this device. Gives back the line left in its place, if any.
async fn delete<S: Store>(
    manager: &mut Manager<S, Registered>,
    dir: &str,
    key: &str,
    thread: &str,
    timestamp: u64,
    everyone: bool,
) -> Result<Option<Event>> {
    let to = find_thread(dir, key, thread).await?;
    if !everyone {
        // Its changes are kept apart, and would bring it back.
        let mut store = manager.store().clone();
        let mut changes = Vec::new();
        for content in store.messages(&to, ..).await?.flatten() {
            if let ContentBody::EditMessage(EditMessage { target_sent_timestamp: Some(t), .. })
            | ContentBody::SynchronizeMessage(SyncMessage {
                content: Some(SyncContent::Sent(Sent { edit_message: Some(EditMessage { target_sent_timestamp: Some(t), .. }), .. })),
                ..
            }) = &content.body
                && *t == timestamp
            {
                changes.push(content.metadata.client_timestamp.timestamp_millis() as u64);
            }
        }
        for at in changes.into_iter().chain([timestamp]) {
            store.delete_message(&to, at).await?;
        }
        return Ok(None);
    }
    let (original, _) = own_message(manager, &to, timestamp).await?;
    let at = now()?;
    let mut message = DataMessage { delete: Some(Delete { target_sent_timestamp: Some(timestamp) }), timestamp: Some(at), ..Default::default() };
    in_group(manager, &to, &mut message).await;
    send_body(manager, &to, message.clone().into(), at).await?;
    let content = Content { metadata: original.metadata, body: message.into() };
    Ok(deleted(manager, &content, to, timestamp).await)
}

/// How many of a chat's latest messages tell the other devices up to where it was deleted, as
/// Signal sends them.
const DELETE_UP_TO: usize = 5;

/// Deletes chat `thread` from the store, and asks the account's other devices (the phone) to
/// delete it too, as Signal Desktop does: they delete up to its latest messages.
async fn delete_chat<S: Store>(manager: &mut Manager<S, Registered>, dir: &str, key: &str, thread: &str) -> Result<()> {
    let to = find_thread(dir, key, thread).await?;
    let mut latest: Vec<(u64, ServiceId)> = Vec::new();
    for content in manager.store().messages(&to, ..).await?.flatten() {
        if matches!(
            &content.body,
            ContentBody::DataMessage(_) | ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(_)), .. })
        ) {
            latest.push((content.metadata.client_timestamp.timestamp_millis() as u64, content.metadata.sender));
        }
    }
    latest.sort_by_key(|m| std::cmp::Reverse(m.0));
    let most_recent_messages = latest
        .into_iter()
        .take(DELETE_UP_TO)
        .map(|(at, author)| AddressableMessage {
            author: Some(addressable_message::Author::AuthorServiceId(author.service_id_string())),
            sent_timestamp: Some(at),
        })
        .collect();
    let conversation = match &to {
        Thread::Contact(id) => conversation_identifier::Identifier::ThreadServiceId(id.service_id_string()),
        Thread::Group(key) => conversation_identifier::Identifier::ThreadGroupId(
            GroupSecretParams::derive_from_master_key(GroupMasterKey::new(*key)).get_group_identifier().to_vec(),
        ),
    };
    let delete = delete_for_me::ConversationDelete {
        conversation: Some(ConversationIdentifier { identifier: Some(conversation) }),
        most_recent_messages,
        is_full_delete: Some(true),
        most_recent_non_expiring_messages: Vec::new(),
    };
    let mut store = manager.store().clone();
    store.clear_thread(&to).await?;
    let sync = SyncMessage {
        content: Some(SyncContent::DeleteForMe(DeleteForMe { conversation_deletes: vec![delete], ..Default::default() })),
        ..Default::default()
    };
    let me = manager.registration_data().service_ids.aci();
    manager.send_message(me, sync, now()?).await.map_err(|e| anyhow!("{e}"))
}

/// The chats another device of the account's deleted: gone from the store here too, by the id
/// Merel knows them by.
async fn chats_deleted<S: Store>(manager: &Manager<S, Registered>, d: &DeleteForMe) -> Vec<String> {
    let ids = d.conversation_deletes.iter().map(|c| &c.conversation).chain(d.local_only_conversation_deletes.iter().map(|c| &c.conversation));
    let mut out = Vec::new();
    for id in ids.flatten().filter_map(|c| c.identifier.as_ref()) {
        let thread = match id {
            conversation_identifier::Identifier::ThreadServiceId(s) => ServiceId::parse_from_service_id_string(s).map(Thread::Contact),
            conversation_identifier::Identifier::ThreadServiceIdBinary(b) => ServiceId::parse_from_service_id_binary(b).map(Thread::Contact),
            conversation_identifier::Identifier::ThreadGroupId(g) => group_by_id(manager, g).await.map(Thread::Group),
            conversation_identifier::Identifier::ThreadE164(_) => None,
        };
        let Some(thread) = thread else { continue };
        let mut store = manager.store().clone();
        match store.clear_thread(&thread).await {
            Ok(()) => out.push(thread_id(&thread)),
            Err(e) => eprintln!("merel-signal: a chat deleted on the phone stays: {e}"),
        }
    }
    out
}

/// The master key of the group Signal names by `id`, the hash of its key.
async fn group_by_id<S: Store>(manager: &Manager<S, Registered>, id: &[u8]) -> Option<[u8; 32]> {
    let groups = manager.store().groups().await.ok()?;
    groups.flatten().map(|(key, _)| key).find(|key| GroupSecretParams::derive_from_master_key(GroupMasterKey::new(*key)).get_group_identifier() == id)
}

/// The line a message deleted for everyone leaves, as Signal shows it. The store then holds
/// an empty message in its place, with no sender; this puts the deletion there instead, so
/// the line is still there after a restart. `None` for a message the store never had.
async fn deleted<S: Store>(manager: &Manager<S, Registered>, content: &Content, thread: Thread, target: u64) -> Option<Event> {
    let me = ServiceId::from(manager.registration_data().service_ids.aci());
    let author = content.metadata.sender;
    let mut kept = manager.store().message(&thread, target).await.ok()??;
    let ContentBody::DataMessage(DataMessage { delete: Some(_), .. }) = kept.body else {
        let data = match &content.body {
            ContentBody::DataMessage(d) => d.clone(),
            ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(Sent { message: Some(d), .. })), .. }) => d.clone(),
            _ => return None,
        };
        kept.metadata.sender = author;
        kept.metadata.destination = match thread {
            Thread::Contact(id) if author == me => id,
            _ => me,
        };
        kept.body = data.into();
        if let Err(e) = manager.store().save_message(&thread, kept).await {
            eprintln!("merel-signal: could not keep a deletion: {e}");
        }
        return deleted_line(manager, thread, author, author == me, target).await;
    };
    deleted_line(manager, thread, author, author == me, target).await
}

async fn deleted_line<S: Store>(manager: &Manager<S, Registered>, thread: Thread, author: ServiceId, mine: bool, target: u64) -> Option<Event> {
    let (thread_name, group) = name_of(manager, &thread).await;
    Some(Event::Message {
        thread: thread_id(&thread),
        thread_name,
        group,
        sender: author.service_id_string(),
        sender_name: contact_name(manager, &author).await,
        mine,
        timestamp: target,
        text: String::new(),
        attachments: Vec::new(),
        quote: None,
        call: None,
        edited: 0,
        deleted: true,
    })
}
