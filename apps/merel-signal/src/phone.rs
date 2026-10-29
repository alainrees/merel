//! Calls as the main loop sees them. The calls themselves run in `merel-signal-calls`, next to
//! this program (see `wire.rs` for why): this side starts it, carries its call messages over
//! Signal, tells Merel how calls go, and logs them. Without that program Merel cannot call, and
//! says so.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use presage::Manager;
use presage::libsignal_service::content::Content;
use presage::libsignal_service::protocol::{IdentityKeyStore as _, ProtocolAddress, ServiceId};
use presage::manager::Registered;
use presage::proto::sync_message::{CallEvent, call_event};
use presage::proto::{CallMessage, call_message};
use presage::store::{ContentExt as _, Store};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::wire::{FromCalls, Hangup, Offer, Opaque, Phase, Relay, Signalling, ToCalls};
use crate::{Event, Ringing, contact_name, logged_call};

/// The calls program: next to this one, unless `MEREL_SIGNAL_CALLS` names another.
fn program() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MEREL_SIGNAL_CALLS") {
        return Some(PathBuf::from(path));
    }
    let name = if cfg!(windows) { "merel-signal-calls.exe" } else { "merel-signal-calls" };
    let path = std::env::current_exe().ok()?.parent()?.join(name);
    path.is_file().then_some(path)
}

/// A call Merel takes part in, for the call log when it ends.
struct Ongoing {
    peer: ServiceId,
    outgoing: bool,
    connected: bool,
    started: u64,
}

pub struct Phone {
    child: Option<(Child, ChildStdin)>,
    /// Set once the calls program said sound works.
    ready: bool,
    ongoing: HashMap<u64, Ongoing>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
}

/// An identity key as RingRTC wants it: without the leading key-type byte, as Signal's apps do.
fn bare_key(serialized: &[u8]) -> Vec<u8> {
    serialized.get(1..).unwrap_or_default().to_vec()
}

impl Phone {
    /// Starts the calls program, if it is there; what it says comes out of the receiver.
    pub fn start() -> (Phone, UnboundedReceiver<FromCalls>) {
        let (tx, rx) = unbounded_channel();
        let child = program().and_then(|path| {
            let mut child = Command::new(path).stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().ok()?;
            let stdin = child.stdin.take()?;
            let stdout = child.stdout.take()?;
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    match serde_json::from_str::<FromCalls>(&line) {
                        Ok(said) => {
                            if tx.send(said).is_err() {
                                break;
                            }
                        }
                        Err(e) => eprintln!("merel-signal: the calls program said something unknown: {e}"),
                    }
                }
            });
            Some((child, stdin))
        });
        (Phone { child, ready: false, ongoing: HashMap::new() }, rx)
    }

    /// Whether calls can be made from here.
    pub fn ready(&self) -> bool {
        self.ready
    }

    async fn tell(&mut self, asked: &ToCalls) -> Result<()> {
        let (_, stdin) = self.child.as_mut().filter(|_| self.ready).ok_or_else(|| anyhow!("calling is not available on this computer"))?;
        let mut line = serde_json::to_string(asked)?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    pub async fn call<S: Store>(&mut self, manager: &Manager<S, Registered>, thread: &str) -> Result<()> {
        let Some(peer) = ServiceId::parse_from_service_id_string(thread) else {
            bail!("only a chat with one person can be called");
        };
        let call_id = uuid::Uuid::new_v4().as_u64_pair().0;
        let local_device = u32::from(manager.device_id());
        self.tell(&ToCalls::Call { peer: peer.service_id_string(), call_id, local_device }).await?;
        self.ongoing.insert(call_id, Ongoing { peer, outgoing: true, connected: false, started: now() });
        Ok(())
    }

    pub async fn accept(&mut self, call_id: u64) -> Result<()> {
        self.tell(&ToCalls::Accept { call_id }).await
    }

    pub async fn hang_up(&mut self) -> Result<()> {
        self.tell(&ToCalls::HangUp).await
    }

    pub async fn mute(&mut self, on: bool) -> Result<()> {
        self.tell(&ToCalls::Mute { on }).await
    }

    /// A call message from someone: the calls program gets it with what it needs to check it.
    pub async fn received<S: Store>(&mut self, manager: &Manager<S, Registered>, content: &Content, message: &CallMessage) {
        if !self.ready {
            return;
        }
        let peer = content.metadata.sender;
        let store = manager.store().aci_protocol_store();
        let address = ProtocolAddress::new(peer.service_id_string(), content.metadata.sender_device);
        let (Ok(Some(theirs)), Ok(mine)) = (store.get_identity(&address).await, store.get_identity_key_pair().await) else {
            eprintln!("merel-signal: a call without known identity keys is ignored");
            return;
        };
        if let Some(id) = message.offer.as_ref().and_then(|o| o.id) {
            self.ongoing.insert(id, Ongoing { peer, outgoing: false, connected: false, started: content.timestamp() });
        }
        let sent = content.metadata.server_timestamp.timestamp_millis().max(0) as u64;
        let asked = ToCalls::Received {
            peer: peer.service_id_string(),
            device: u32::from(content.metadata.sender_device),
            my_device: u32::from(manager.device_id()),
            age_ms: now().saturating_sub(sent),
            peer_key: bare_key(&theirs.serialize()),
            my_key: bare_key(&mine.identity_key().serialize()),
            message: from_proto(message),
        };
        if let Err(e) = self.tell(&asked).await {
            eprintln!("merel-signal: {e:#}");
        }
    }

    /// Does what the calls program asked; returns what Merel should hear.
    pub async fn handle<S: Store>(&mut self, said: FromCalls, manager: &mut Manager<S, Registered>, ringing: &mut Ringing) -> Vec<Event> {
        match said {
            FromCalls::Ready => {
                self.ready = true;
                vec![Event::Calls { available: true }]
            }
            FromCalls::Send { to, call_id, message } => {
                let ok = match ServiceId::parse_from_service_id_string(&to) {
                    Some(to) => match manager.send_message(to, to_proto(&message), now()).await {
                        Ok(()) => true,
                        Err(e) => {
                            eprintln!("merel-signal: a call message did not go out: {e}");
                            false
                        }
                    },
                    None => false,
                };
                if let Err(e) = self.tell(&ToCalls::Sent { call_id, ok }).await {
                    eprintln!("merel-signal: {e:#}");
                }
                Vec::new()
            }
            FromCalls::Proceed { call_id } => {
                let relays = manager
                    .calling_relays()
                    .await
                    .unwrap_or_else(|e| {
                        eprintln!("merel-signal: no relays for the call: {e}");
                        Vec::new()
                    })
                    .into_iter()
                    .map(|r| Relay {
                        username: r.username,
                        password: r.password,
                        hostname: r.hostname.unwrap_or_default(),
                        urls: r.urls,
                        urls_with_ips: r.urls_with_ips,
                    })
                    .collect();
                if let Err(e) = self.tell(&ToCalls::Proceed { call_id, relays }).await {
                    eprintln!("merel-signal: {e:#}");
                }
                Vec::new()
            }
            FromCalls::State { peer, call_id, state } => {
                let mut events = Vec::new();
                let Some(id) = ServiceId::parse_from_service_id_string(&peer) else { return events };
                events.push(Event::Call { thread: peer, name: contact_name(manager, &id).await, call_id, state });
                if state == Phase::Connected {
                    ringing.taken(call_id);
                    if let Some(call) = self.ongoing.get_mut(&call_id) {
                        call.connected = true;
                    }
                }
                // Another device logs the calls it answered or declined itself.
                if state.is_over()
                    && let Some(call) = self.ongoing.remove(&call_id)
                    && (call.outgoing || call.connected)
                    && let Some(event) = log(manager, call_id, &call).await
                {
                    events.push(event);
                }
                events
            }
        }
    }

    /// The calls program stopped: no more calls until the next start.
    pub fn stopped(&mut self) -> Option<Event> {
        let was = self.ready;
        self.ready = false;
        self.child = None;
        was.then_some(Event::Calls { available: false })
    }
}

/// Logs a call Merel made or answered, here and on the account's other devices.
async fn log<S: Store>(manager: &mut Manager<S, Registered>, call_id: u64, call: &Ongoing) -> Option<Event> {
    let event = CallEvent {
        conversation_id: Some(call.peer.service_id_binary()),
        call_id: Some(call_id),
        timestamp: Some(call.started),
        r#type: Some(call_event::Type::AudioCall as i32),
        direction: Some(if call.outgoing { call_event::Direction::Outgoing } else { call_event::Direction::Incoming } as i32),
        event: Some(if call.connected { call_event::Event::Accepted } else { call_event::Event::NotAccepted } as i32),
    };
    match manager.send_call_event(event.clone()).await {
        Ok(content) => logged_call(manager, &content, &event).await,
        Err(e) => {
            eprintln!("merel-signal: the call was not logged: {e}");
            None
        }
    }
}

/// A call message as the calls program reads it.
fn from_proto(m: &CallMessage) -> Signalling {
    Signalling {
        destination_device_id: m.destination_device_id,
        offer: m
            .offer
            .as_ref()
            .and_then(|o| Some(Offer { id: o.id?, video: o.r#type() == call_message::offer::Type::OfferVideoCall, opaque: o.opaque.clone()? })),
        answer: m.answer.as_ref().and_then(|a| Some(Opaque { id: a.id?, opaque: a.opaque.clone()? })),
        ice: m.ice_update.iter().filter_map(|i| Some(Opaque { id: i.id?, opaque: i.opaque.clone()? })).collect(),
        hangup: m.hangup.as_ref().and_then(|h| Some(Hangup { id: h.id?, kind: h.r#type.unwrap_or_default(), device_id: h.device_id })),
        busy: m.busy.as_ref().and_then(|b| b.id),
    }
}

/// A call message as Signal carries it.
fn to_proto(s: &Signalling) -> CallMessage {
    CallMessage {
        destination_device_id: s.destination_device_id,
        offer: s.offer.as_ref().map(|o| call_message::Offer {
            id: Some(o.id),
            r#type: Some(if o.video { call_message::offer::Type::OfferVideoCall } else { call_message::offer::Type::OfferAudioCall } as i32),
            opaque: Some(o.opaque.clone()),
        }),
        answer: s.answer.as_ref().map(|a| call_message::Answer { id: Some(a.id), opaque: Some(a.opaque.clone()) }),
        ice_update: s.ice.iter().map(|i| call_message::IceUpdate { id: Some(i.id), opaque: Some(i.opaque.clone()) }).collect(),
        hangup: s.hangup.as_ref().map(|h| call_message::Hangup { id: Some(h.id), r#type: Some(h.kind), device_id: h.device_id }),
        busy: s.busy.map(|id| call_message::Busy { id: Some(id) }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_message_survives_the_way_to_the_calls_program_and_back() {
        let s = Signalling {
            destination_device_id: Some(2),
            offer: Some(Offer { id: 7, video: false, opaque: vec![1, 2, 3] }),
            answer: None,
            ice: vec![Opaque { id: 7, opaque: vec![4] }, Opaque { id: 7, opaque: vec![5] }],
            hangup: Some(Hangup { id: 7, kind: 1, device_id: Some(3) }),
            busy: Some(7),
        };
        assert_eq!(from_proto(&to_proto(&s)), s);
        let line = serde_json::to_string(&s).unwrap_or_default();
        assert!(line.contains("\"010203\""), "{line}");
    }

    #[test]
    fn keys_lose_their_type_byte() {
        assert_eq!(bare_key(&[5, 1, 2]), vec![1, 2]);
        assert!(bare_key(&[]).is_empty());
    }
}
