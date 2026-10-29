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
use futures::channel::oneshot;
use futures::{StreamExt, future, pin_mut};
use presage::Manager;
use presage::libsignal_service::configuration::SignalServers;
use presage::libsignal_service::content::Metadata;
use presage::libsignal_service::content::{Content, ContentBody, DataMessage};
use presage::libsignal_service::protocol::ServiceId;
use presage::manager::Registered;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::proto::data_message::Reaction;
use presage::proto::sync_message::{CallEvent, Content as SyncContent, Sent, call_event};
use presage::proto::{CallMessage, EditMessage, SyncMessage, call_message};
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
        /// Set when this is a call rather than a message: the text is then empty.
        #[serde(skip_serializing_if = "Option::is_none")]
        call: Option<Call>,
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
    Send {
        thread: String,
        text: String,
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

/// Most calls kept ringing at once; a caller that never hangs up is forgotten.
const MAX_RINGING: usize = 64;

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
    let mut ringing = Ringing::default();
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
                    let shown = match &content.body {
                        ContentBody::CallMessage(call) => ringing.hear(&manager, &content, call).await,
                        _ => describe(&manager, &content).await,
                    };
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
                                call: None,
                            });
                            emit(&Event::Sent { thread, timestamp });
                        }
                        Err(e) => emit(&Event::Error { message: format!("{e:#}") }),
                    },
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
async fn describe<S: Store>(manager: &Manager<S, Registered>, content: &Content) -> Option<Event> {
    if let ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::CallEvent(event)), .. }) = &content.body {
        return logged_call(manager, content, event).await;
    }
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
    let DataMessage { body, attachments, reaction, group_call_update, .. } = data;
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
    if group_call_update.is_some() {
        let call = Call { id: content.timestamp(), kind: CallKind::Group, state: CallState::Started };
        return chat_call(manager, Thread::try_from(content).ok()?, &content.metadata.sender, mine, call, content.timestamp()).await;
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
        call: None,
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
        call: Some(call),
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

async fn send<S: Store>(manager: &mut Manager<S, Registered>, dir: &str, key: &str, thread: &str, text: &str) -> Result<(Thread, u64)> {
    send_data(manager, dir, key, thread, DataMessage { body: Some(text.to_string()), ..Default::default() }).await
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
