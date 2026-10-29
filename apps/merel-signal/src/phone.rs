//! Calls as the main loop sees them. Built with the feature `calls`, they go through RingRTC
//! (see [`crate::calls`]); without it this says that calling is not there.

#[cfg(feature = "calls")]
pub use crate::calls::Phase;

#[cfg(not(feature = "calls"))]
pub enum Note {}

/// A call's phase as Merel shows it.
#[cfg(not(feature = "calls"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Phase {}

#[cfg(feature = "calls")]
mod with_calls {
    use std::collections::HashMap;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use anyhow::{Result, anyhow, bail};
    use presage::Manager;
    use presage::libsignal_service::content::Content;
    use presage::libsignal_service::protocol::{IdentityKeyStore as _, ProtocolAddress, ServiceId};
    use presage::manager::Registered;
    use presage::proto::CallMessage;
    use presage::proto::sync_message::{CallEvent, call_event};
    use presage::store::{ContentExt as _, Store};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use crate::calls::{Calls, Note, Phase, Received, bare_key};
    use crate::{Event, Ringing, contact_name, logged_call};

    /// A call Merel takes part in, for the call log when it ends.
    struct Ongoing {
        peer: ServiceId,
        outgoing: bool,
        connected: bool,
        started: u64,
    }

    pub struct Phone {
        calls: Option<Calls>,
        ongoing: HashMap<u64, Ongoing>,
    }

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
    }

    impl Phone {
        pub fn start() -> (Phone, UnboundedReceiver<Note>) {
            let (tx, rx) = unbounded_channel();
            let calls = match Calls::start(tx) {
                Ok(calls) => Some(calls),
                Err(e) => {
                    eprintln!("merel-signal: calling is not available: {e:#}");
                    None
                }
            };
            (Phone { calls, ongoing: HashMap::new() }, rx)
        }

        fn calls(&mut self) -> Result<&mut Calls> {
            self.calls.as_mut().ok_or_else(|| anyhow!("calling is not available on this computer"))
        }

        pub async fn call<S: Store>(&mut self, manager: &Manager<S, Registered>, thread: &str) -> Result<()> {
            let Some(peer) = ServiceId::parse_from_service_id_string(thread) else {
                bail!("only a chat with one person can be called");
            };
            let device = u32::from(manager.device_id());
            let call_id = self.calls()?.call(&peer.service_id_string(), device);
            self.ongoing.insert(call_id, Ongoing { peer, outgoing: true, connected: false, started: now() });
            Ok(())
        }

        pub fn accept(&mut self, call_id: u64) {
            if let Some(calls) = &mut self.calls {
                calls.accept(call_id);
            }
        }

        pub fn hang_up(&mut self) {
            if let Some(calls) = &mut self.calls {
                calls.hang_up();
            }
        }

        pub fn mute(&mut self, on: bool) {
            if let Some(calls) = &mut self.calls {
                calls.mute(on);
            }
        }

        /// A call message from someone: RingRTC gets it with what it needs to check it.
        pub async fn received<S: Store>(&mut self, manager: &Manager<S, Registered>, content: &Content, message: &CallMessage) {
            if self.calls.is_none() {
                return;
            }
            let peer = content.metadata.sender;
            let device = u32::from(content.metadata.sender_device);
            let store = manager.store().aci_protocol_store();
            let address = ProtocolAddress::new(peer.service_id_string(), content.metadata.sender_device);
            let (Ok(Some(theirs)), Ok(mine)) = (store.get_identity(&address).await, store.get_identity_key_pair().await) else {
                eprintln!("merel-signal: a call without known identity keys is ignored");
                return;
            };
            if message.offer.is_some()
                && let Some(id) = message.offer.as_ref().and_then(|o| o.id)
            {
                self.ongoing.insert(id, Ongoing { peer, outgoing: false, connected: false, started: content.timestamp() });
            }
            let sent = content.metadata.server_timestamp.timestamp_millis().max(0) as u64;
            let from = Received {
                device,
                my_device: u32::from(manager.device_id()),
                age: Duration::from_millis(now().saturating_sub(sent)),
                peer_key: bare_key(&theirs.serialize()),
                my_key: bare_key(&mine.identity_key().serialize()),
            };
            if let Some(calls) = &mut self.calls {
                calls.received(&peer.service_id_string(), from, message);
            }
        }

        /// Does what RingRTC asked; returns what Merel should hear.
        pub async fn handle<S: Store>(&mut self, note: Note, manager: &mut Manager<S, Registered>, ringing: &mut Ringing) -> Vec<Event> {
            match note {
                Note::Send { to, call_id, message } => {
                    let ok = match ServiceId::parse_from_service_id_string(&to) {
                        Some(to) => match manager.send_message(to, message, now()).await {
                            Ok(()) => true,
                            Err(e) => {
                                eprintln!("merel-signal: a call message did not go out: {e}");
                                false
                            }
                        },
                        None => false,
                    };
                    if let Some(calls) = &mut self.calls {
                        calls.sent(call_id, ok);
                    }
                    Vec::new()
                }
                Note::Proceed { call_id } => {
                    let relays = manager.calling_relays().await.unwrap_or_else(|e| {
                        eprintln!("merel-signal: no relays for the call: {e}");
                        Vec::new()
                    });
                    // Always through a relay: the other side never learns this computer's address.
                    if let Some(calls) = &mut self.calls
                        && let Err(e) = calls.proceed(call_id, &relays, true)
                    {
                        eprintln!("merel-signal: the call cannot go on: {e:#}");
                        calls.hang_up();
                    }
                    Vec::new()
                }
                Note::State { peer, call_id, state } => {
                    let mut events = Vec::new();
                    let Some(id) = ServiceId::parse_from_service_id_string(&peer) else { return events };
                    events.push(Event::Call { thread: peer, name: contact_name(manager, &id).await, call_id, state });
                    match state {
                        Phase::Connected => {
                            ringing.taken(call_id);
                            if let Some(call) = self.ongoing.get_mut(&call_id) {
                                call.connected = true;
                            }
                        }
                        Phase::Ended | Phase::Failed | Phase::Declined | Phase::AnsweredElsewhere => {
                            if let Some(calls) = &mut self.calls {
                                calls.ended();
                            }
                            // Another device logs the calls it answered or declined itself.
                            if let Some(call) = self.ongoing.remove(&call_id)
                                && (call.outgoing || call.connected)
                                && let Some(event) = self.log(manager, call_id, &call).await
                            {
                                events.push(event);
                            }
                        }
                        _ => {}
                    }
                    events
                }
            }
        }

        /// Logs a call Merel made or answered, here and on the account's other devices.
        async fn log<S: Store>(&self, manager: &mut Manager<S, Registered>, call_id: u64, call: &Ongoing) -> Option<Event> {
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
    }
}

#[cfg(feature = "calls")]
pub use with_calls::Phone;

#[cfg(not(feature = "calls"))]
pub use without_calls::Phone;

#[cfg(not(feature = "calls"))]
mod without_calls {
    use anyhow::Result;
    use presage::Manager;
    use presage::libsignal_service::content::Content;
    use presage::manager::Registered;
    use presage::proto::CallMessage;
    use presage::store::Store;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::Note;
    use crate::{Event, Ringing};

    /// Without RingRTC: calls show in the chats, but Merel cannot take part.
    pub struct Phone;

    impl Phone {
        pub fn start() -> (Phone, UnboundedReceiver<Note>) {
            let (_, rx) = unbounded_channel();
            (Phone, rx)
        }

        pub async fn call<S: Store>(&mut self, _: &Manager<S, Registered>, _: &str) -> Result<()> {
            anyhow::bail!("this build of Merel cannot call over Signal")
        }

        pub fn accept(&mut self, _: u64) {}

        pub fn hang_up(&mut self) {}

        pub fn mute(&mut self, _: bool) {}

        pub async fn received<S: Store>(&mut self, _: &Manager<S, Registered>, _: &Content, _: &CallMessage) {}

        pub async fn handle<S: Store>(&mut self, note: Note, _: &mut Manager<S, Registered>, _: &mut Ringing) -> Vec<Event> {
            match note {}
        }
    }
}
