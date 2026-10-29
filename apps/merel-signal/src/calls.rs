//! Calls with one person over Signal, through Signal's own calling library RingRTC.
//!
//! Signal does not call over plain WebRTC: the keys for a call travel inside the end-to-end
//! encrypted Signal messages (offer, answer, ICE, hangup), so only RingRTC can talk to Signal's
//! apps. RingRTC plays and records sound itself, so the voice never passes through Merel.
//!
//! RingRTC calls back on its own threads. Everything it wants done (send a message, go on with a
//! call, tell Merel) comes to the main loop as a [`Note`], which does it with the Signal account.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::{Result, anyhow};
use presage::libsignal_service::websocket::calling::TurnServer;
use presage::proto::{CallMessage, call_message};
use ringrtc::common::{CallConfig, CallId, CallMediaType, DeviceId};
use ringrtc::core::call_manager::CallManager;
use ringrtc::core::{group_call, signaling};
use ringrtc::lite::{http, sfu};
use ringrtc::native::{CallState, CallStateHandler, GroupUpdate, GroupUpdateHandler, NativeCallContext, NativePlatform, SignalingSender};
use ringrtc::webrtc::media::{AudioTrack, VideoFrame, VideoSink};
use ringrtc::webrtc::peer_connection::AudioLevel;
use ringrtc::webrtc::peer_connection_factory::{AudioConfig, IceServer, PeerConnectionFactory};
use ringrtc::webrtc::peer_connection_observer::NetworkRoute;
use tokio::sync::mpsc::UnboundedSender;

/// What RingRTC asks of the main loop.
pub enum Note {
    /// Send this to `to` (a service id), to one of their devices or all of them.
    Send { to: String, call_id: u64, message: CallMessage },
    /// A call is set up: it may go on once it has the relays to connect through.
    Proceed { call_id: u64 },
    /// How a call is going, for Merel.
    State { peer: String, call_id: u64, state: Phase },
}

/// A call's phase as Merel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Someone calls: Merel rings and offers to answer.
    Incoming,
    /// The account calls someone.
    Outgoing,
    /// The other side rings.
    Ringing,
    Connecting,
    Connected,
    /// Answered on another of the account's devices.
    AnsweredElsewhere,
    /// Declined on another device, or they were busy.
    Declined,
    Ended,
    /// Could not connect: no network, no answer in time, or something broke.
    Failed,
}

/// The calls of one Signal account.
pub struct Calls {
    manager: CallManager<NativePlatform>,
    factory: PeerConnectionFactory,
    /// The microphone, while a call goes on.
    microphone: Option<AudioTrack>,
}

impl Calls {
    pub fn start(notes: UnboundedSender<Note>) -> Result<Calls> {
        let mut factory = PeerConnectionFactory::new(&AudioConfig::default(), false, "", None)?;
        use_default_devices(&mut factory);
        let platform = NativePlatform::new(
            factory.clone(),
            Box::new(Relay(notes.clone())),
            // Messages go out on their own time; the main loop says when one went.
            false,
            Box::new(Relay(notes.clone())),
            Box::new(Relay(notes)),
        );
        let manager = CallManager::new(platform, http::DelegatingClient::new(NoHttp))?;
        Ok(Calls { manager, factory, microphone: None })
    }

    /// Calls `peer` (a service id): sound only for now.
    pub fn call(&mut self, peer: &str, local_device: u32) -> u64 {
        let id = CallId::new(uuid::Uuid::new_v4().as_u64_pair().0);
        self.manager.create_outgoing_call(peer.to_string(), id, CallMediaType::Audio, local_device);
        id.as_u64()
    }

    /// Goes on with a call that is set up, connecting through Signal's relays. `hide_ip` sends
    /// all sound through a relay, so the other side never learns the computer's address.
    pub fn proceed(&mut self, call_id: u64, relays: &[TurnServer], hide_ip: bool) -> Result<()> {
        let microphone = self.factory.create_outgoing_audio_track()?;
        let camera = self.factory.create_outgoing_video_track(&self.factory.create_outgoing_video_source()?)?;
        // Video stays off: the camera track is there because RingRTC wants one.
        camera.set_enabled(false);
        let mut servers = Vec::new();
        for relay in relays {
            if !relay.urls_with_ips.is_empty() {
                servers.push(IceServer::new(
                    relay.username.clone(),
                    relay.password.clone(),
                    relay.hostname.clone().unwrap_or_default(),
                    relay.urls_with_ips.clone(),
                ));
            }
            if !relay.urls.is_empty() {
                servers.push(IceServer::new(relay.username.clone(), relay.password.clone(), String::new(), relay.urls.clone()));
            }
        }
        let context = NativeCallContext::new(hide_ip, servers, microphone.clone(), camera, Box::new(NoVideo));
        self.microphone = Some(microphone);
        self.manager.proceed(CallId::new(call_id), context, CallConfig::default(), None);
        Ok(())
    }

    pub fn accept(&mut self, call_id: u64) {
        self.manager.accept_call(CallId::new(call_id));
    }

    /// Ends the call going on, or declines the one ringing.
    pub fn hang_up(&mut self) {
        self.manager.hangup();
    }

    pub fn mute(&mut self, on: bool) {
        if let Some(microphone) = &self.microphone {
            microphone.set_enabled(!on);
        }
    }

    pub fn ended(&mut self) {
        self.microphone = None;
    }

    pub fn sent(&mut self, call_id: u64, ok: bool) {
        match ok {
            true => self.manager.message_sent(CallId::new(call_id)),
            false => self.manager.message_send_failure(CallId::new(call_id)),
        }
    }

    /// Passes a call message from `peer` on to RingRTC. The identity keys are the two
    /// accounts' public identity keys, which RingRTC mixes into the call's keys.
    pub fn received(&mut self, peer: &str, from: Received, message: &CallMessage) {
        let peer = peer.to_string();
        let Received { device, my_device, age, peer_key, my_key } = from;
        if let Some(to) = message.destination_device_id
            && to != my_device
        {
            return;
        }
        if let Some(offer) = &message.offer
            && let (Some(id), Some(opaque)) = (offer.id, offer.opaque.clone())
        {
            let media = match offer.r#type() {
                call_message::offer::Type::OfferVideoCall => CallMediaType::Video,
                call_message::offer::Type::OfferAudioCall => CallMediaType::Audio,
            };
            let Ok(offer) = signaling::Offer::new(media, opaque) else { return };
            let received = signaling::ReceivedOffer {
                offer,
                age,
                sender_device_id: device,
                receiver_device_id: my_device,
                sender_identity_key: peer_key,
                receiver_identity_key: my_key,
            };
            self.manager.received_offer(peer, CallId::new(id), received);
        } else if let Some(answer) = &message.answer
            && let (Some(id), Some(opaque)) = (answer.id, answer.opaque.clone())
        {
            let Ok(answer) = signaling::Answer::new(opaque) else { return };
            let received =
                signaling::ReceivedAnswer { answer, sender_device_id: device, sender_identity_key: peer_key, receiver_identity_key: my_key };
            self.manager.received_answer(peer, CallId::new(id), received);
        } else if let Some(id) = message.ice_update.first().and_then(|i| i.id) {
            let candidates = message.ice_update.iter().filter_map(|i| i.opaque.clone()).map(signaling::IceCandidate::new).collect();
            let received = signaling::ReceivedIce { ice: signaling::Ice { candidates }, sender_device_id: device };
            self.manager.received_ice(peer, CallId::new(id), received);
        } else if let Some(hangup) = &message.hangup
            && let Some(id) = hangup.id
        {
            let Some(kind) = signaling::HangupType::from_i32(hangup.r#type.unwrap_or_default()) else { return };
            let hangup = signaling::Hangup::from_type_and_device_id(kind, hangup.device_id.unwrap_or_default());
            self.manager.received_hangup(peer, CallId::new(id), signaling::ReceivedHangup { hangup, sender_device_id: device });
        } else if let Some(busy) = &message.busy
            && let Some(id) = busy.id
        {
            self.manager.received_busy(peer, CallId::new(id), signaling::ReceivedBusy { sender_device_id: device });
        }
    }
}

/// Where a call message came from.
pub struct Received {
    pub device: DeviceId,
    pub my_device: DeviceId,
    /// How long ago it was sent: RingRTC ignores offers that are too old.
    pub age: Duration,
    pub peer_key: Vec<u8>,
    pub my_key: Vec<u8>,
}

/// An identity key as RingRTC wants it: without the leading key-type byte, as Signal's apps do.
pub fn bare_key(serialized: &[u8]) -> Vec<u8> {
    serialized.get(1..).unwrap_or_default().to_vec()
}

/// The system's default microphone and speaker, as calls apps use them.
fn use_default_devices(factory: &mut PeerConnectionFactory) {
    if let Ok(devices) = factory.get_audio_playout_devices()
        && let Some(device) = devices.default.default_for_voip()
        && let Err(e) = factory.set_audio_playout_device_by_id(&device.unique_id, 0)
    {
        eprintln!("merel-signal: no speaker: {e}");
    }
    if let Ok(devices) = factory.get_audio_recording_devices()
        && let Some(device) = devices.default.default_for_voip()
        && let Err(e) = factory.set_audio_recording_device_by_id(&device.unique_id, 0)
    {
        eprintln!("merel-signal: no microphone: {e}");
    }
}

/// The message to send for what RingRTC says.
fn to_proto(call_id: u64, receiver: Option<DeviceId>, message: signaling::Message) -> CallMessage {
    let id = Some(call_id);
    let mut out = CallMessage { destination_device_id: receiver, ..Default::default() };
    match message {
        signaling::Message::Offer(offer) => {
            let kind = match offer.call_media_type {
                CallMediaType::Video => call_message::offer::Type::OfferVideoCall,
                CallMediaType::Audio => call_message::offer::Type::OfferAudioCall,
            };
            out.offer = Some(call_message::Offer { id, r#type: Some(kind as i32), opaque: Some(offer.opaque) });
        }
        signaling::Message::Answer(answer) => out.answer = Some(call_message::Answer { id, opaque: Some(answer.opaque) }),
        signaling::Message::Ice(ice) => {
            out.ice_update = ice.candidates.into_iter().map(|c| call_message::IceUpdate { id, opaque: Some(c.opaque) }).collect();
        }
        signaling::Message::Hangup(hangup) => {
            let (kind, device) = hangup.to_type_and_device_id();
            out.hangup = Some(call_message::Hangup { id, r#type: Some(kind as i32), device_id: device });
        }
        signaling::Message::Busy => out.busy = Some(call_message::Busy { id }),
    }
    out
}

/// RingRTC's callbacks, passed on to the main loop.
struct Relay(UnboundedSender<Note>);

impl Relay {
    fn note(&self, note: Note) -> ringrtc::common::Result<()> {
        self.0.send(note).map_err(|_| anyhow!("merel-signal stopped"))
    }
}

impl SignalingSender for Relay {
    fn send_signaling(
        &self,
        recipient: &str,
        call_id: CallId,
        receiver: Option<DeviceId>,
        message: signaling::Message,
    ) -> ringrtc::common::Result<()> {
        let call_id = call_id.as_u64();
        self.note(Note::Send { to: recipient.to_string(), call_id, message: to_proto(call_id, receiver, message) })
    }

    // Group calls are not there yet.
    fn send_call_message(&self, _: sfu::UserId, _: Vec<u8>, _: group_call::SignalingMessageUrgency) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn send_call_message_to_group(
        &self,
        _: group_call::GroupId,
        _: Vec<u8>,
        _: group_call::SignalingMessageUrgency,
        _: HashSet<sfu::UserId>,
    ) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn send_call_message_to_adhoc_group(
        &self,
        _: Vec<u8>,
        _: group_call::SignalingMessageUrgency,
        _: u64,
        _: HashMap<sfu::UserId, Vec<u8>>,
    ) -> ringrtc::common::Result<()> {
        Ok(())
    }
}

impl CallStateHandler for Relay {
    fn handle_call_state(&self, peer: &str, call_id: CallId, state: CallState) -> ringrtc::common::Result<()> {
        use ringrtc::common::CallEndReason as Why;
        let call_id = call_id.as_u64();
        let phase = match state {
            CallState::Incoming(_) => {
                self.note(Note::Proceed { call_id })?;
                Phase::Incoming
            }
            CallState::Outgoing(_) => {
                self.note(Note::Proceed { call_id })?;
                Phase::Outgoing
            }
            CallState::Ringing => Phase::Ringing,
            CallState::Connecting => Phase::Connecting,
            CallState::Connected => Phase::Connected,
            CallState::Ended(why, _) => match why {
                Why::RemoteHangupAccepted => Phase::AnsweredElsewhere,
                Why::RemoteHangupDeclined | Why::RemoteHangupBusy | Why::RemoteBusy => Phase::Declined,
                Why::LocalHangup | Why::RemoteHangup | Why::RemoteHangupNeedPermission | Why::RemoteGlare | Why::RemoteReCall => Phase::Ended,
                _ => Phase::Failed,
            },
            CallState::Rejected(_) => Phase::Failed,
            CallState::Concluded => return Ok(()),
        };
        self.note(Note::State { peer: peer.to_string(), call_id, state: phase })
    }

    fn handle_remote_audio_state(&self, _: &str, _: bool) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn handle_remote_video_state(&self, _: &str, _: bool) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn handle_remote_sharing_screen(&self, _: &str, _: bool) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn handle_network_route(&self, _: &str, _: NetworkRoute) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn handle_audio_levels(&self, _: &str, _: AudioLevel, _: AudioLevel) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn handle_low_bandwidth_for_video(&self, _: &str, _: bool) -> ringrtc::common::Result<()> {
        Ok(())
    }
}

impl GroupUpdateHandler for Relay {
    fn handle_group_update(&self, _: GroupUpdate) -> ringrtc::common::Result<()> {
        Ok(())
    }
}

/// Group calls ask Signal's call servers over HTTP; calls between two people never do.
struct NoHttp;

impl http::Delegate for NoHttp {
    fn send_request(&self, _: u32, _: http::Request) {}
}

/// Video stays off for now.
#[derive(Clone)]
struct NoVideo;

impl VideoSink for NoVideo {
    fn on_video_frame(&self, _: sfu::DemuxId, _: VideoFrame) {}

    fn box_clone(&self) -> Box<dyn VideoSink> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hangup_goes_out_with_its_kind_and_device() {
        let message = to_proto(7, Some(2), signaling::Message::Hangup(signaling::Hangup::AcceptedOnAnotherDevice(3)));
        let hangup = message.hangup.unwrap_or_default();
        assert_eq!((hangup.id, hangup.r#type, hangup.device_id), (Some(7), Some(1), Some(3)));
        assert_eq!(message.destination_device_id, Some(2));
    }

    #[test]
    fn ice_goes_out_as_one_update_per_candidate() {
        let ice = signaling::Ice { candidates: vec![signaling::IceCandidate::new(vec![1]), signaling::IceCandidate::new(vec![2])] };
        let message = to_proto(9, None, signaling::Message::Ice(ice));
        assert_eq!(message.ice_update.len(), 2);
        assert!(message.ice_update.iter().all(|u| u.id == Some(9)));
    }

    #[test]
    fn keys_lose_their_type_byte() {
        assert_eq!(bare_key(&[5, 1, 2]), vec![1, 2]);
        assert!(bare_key(&[]).is_empty());
    }
}
