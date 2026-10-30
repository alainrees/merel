//! `merel-signal-calls`: calls with one person over Signal, through Signal's own calling library
//! RingRTC. `merel-signal` starts it and talks with it in JSON lines (see `wire.rs`).
//!
//! Signal does not call over plain WebRTC: the keys for a call travel inside the end-to-end
//! encrypted Signal messages (offer, answer, ICE, hangup), so only RingRTC can talk to Signal's
//! apps. RingRTC plays and records the sound itself, so the voice never passes through Merel.
//! This program never sees the Signal account: `merel-signal` carries the messages for it.

// Shared with merel-signal, which uses all of it.
#[allow(dead_code)]
#[path = "../wire.rs"]
mod wire;

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use ringrtc::common::{CallConfig, CallId, CallMediaType, DeviceId};
use ringrtc::core::call_manager::CallManager;
use ringrtc::core::{group_call, signaling};
use ringrtc::lite::{http, sfu};
use ringrtc::native::{CallState, CallStateHandler, GroupUpdate, GroupUpdateHandler, NativeCallContext, NativePlatform, SignalingSender};
use ringrtc::webrtc::media::{AudioTrack, VideoFrame, VideoSink};
use ringrtc::webrtc::peer_connection::AudioLevel;
use ringrtc::webrtc::peer_connection_factory::{AudioConfig, IceServer, PeerConnectionFactory};
use ringrtc::webrtc::peer_connection_observer::NetworkRoute;

use crate::wire::{FromCalls, Hangup, Offer, Opaque, Phase, Relay, Signalling, ToCalls};

fn main() {
    let out = Out::default();
    let mut calls = match Calls::start(out.clone()) {
        Ok(calls) => calls,
        Err(e) => {
            eprintln!("merel-signal-calls: calling is not available: {e:#}");
            std::process::exit(1);
        }
    };
    out.say(&FromCalls::Ready);
    for line in std::io::stdin().lock().lines().map_while(std::result::Result::ok) {
        match serde_json::from_str::<ToCalls>(&line) {
            Ok(asked) => calls.handle(asked),
            Err(e) => eprintln!("merel-signal-calls: unknown command: {e}"),
        }
    }
    // merel-signal closed: done.
}

/// Standard output, shared with RingRTC's threads: one line each.
#[derive(Clone, Default)]
struct Out(Arc<Mutex<()>>);

impl Out {
    fn say(&self, said: &FromCalls) {
        let _guard = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Ok(line) = serde_json::to_string(said) {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{line}").and_then(|()| stdout.flush());
        }
    }
}

/// The calls of one Signal account.
struct Calls {
    manager: CallManager<NativePlatform>,
    factory: PeerConnectionFactory,
    /// The microphone, while a call goes on.
    microphone: Option<AudioTrack>,
}

impl Calls {
    fn start(out: Out) -> Result<Calls> {
        let mut factory = PeerConnectionFactory::new(&AudioConfig::default(), false, "", None)?;
        use_default_devices(&mut factory);
        let platform = NativePlatform::new(
            factory.clone(),
            Box::new(Tell(out.clone())),
            // Messages go out on their own time; merel-signal says when one went.
            false,
            Box::new(Tell(out.clone())),
            Box::new(Tell(out)),
        );
        let manager = CallManager::new(platform, http::DelegatingClient::new(NoHttp))?;
        Ok(Calls { manager, factory, microphone: None })
    }

    fn handle(&mut self, asked: ToCalls) {
        match asked {
            ToCalls::Call { peer, call_id, local_device } => {
                self.manager.create_outgoing_call(peer, CallId::new(call_id), CallMediaType::Audio, local_device);
            }
            ToCalls::Accept { call_id } => self.manager.accept_call(CallId::new(call_id)),
            ToCalls::HangUp => {
                self.manager.hangup();
                self.microphone = None;
            }
            ToCalls::Mute { on } => {
                if let Some(microphone) = &self.microphone {
                    microphone.set_enabled(!on);
                }
            }
            ToCalls::Proceed { call_id, relays } => {
                if let Err(e) = self.proceed(call_id, &relays) {
                    eprintln!("merel-signal-calls: the call cannot go on: {e:#}");
                    self.manager.hangup();
                }
            }
            ToCalls::Sent { call_id, ok: true } => self.manager.message_sent(CallId::new(call_id)),
            ToCalls::Sent { call_id, ok: false } => self.manager.message_send_failure(CallId::new(call_id)),
            ToCalls::Received { peer, device, my_device, age_ms, peer_key, my_key, message } => {
                self.received(peer, device, my_device, Duration::from_millis(age_ms), peer_key, my_key, message);
            }
        }
    }

    /// Goes on with a call that is set up. All sound goes through Signal's relays (`hide_ip`),
    /// so the other side never learns the computer's address.
    fn proceed(&mut self, call_id: u64, relays: &[Relay]) -> Result<()> {
        let microphone = self.factory.create_outgoing_audio_track()?;
        let camera = self.factory.create_outgoing_video_track(&self.factory.create_outgoing_video_source()?)?;
        // Video stays off: the camera track is there because RingRTC wants one.
        camera.set_enabled(false);
        let mut servers = Vec::new();
        for relay in relays {
            if !relay.urls_with_ips.is_empty() {
                servers.push(IceServer::new(relay.username.clone(), relay.password.clone(), relay.hostname.clone(), relay.urls_with_ips.clone()));
            }
            if !relay.urls.is_empty() {
                servers.push(IceServer::new(relay.username.clone(), relay.password.clone(), String::new(), relay.urls.clone()));
            }
        }
        let context = NativeCallContext::new(true, servers, microphone.clone(), camera, Box::new(NoVideo));
        self.microphone = Some(microphone);
        self.manager.proceed(CallId::new(call_id), context, CallConfig::default(), None);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn received(
        &mut self,
        peer: String,
        device: DeviceId,
        my_device: DeviceId,
        age: Duration,
        peer_key: Vec<u8>,
        my_key: Vec<u8>,
        message: Signalling,
    ) {
        if message.destination_device_id.is_some_and(|to| to != my_device) {
            return;
        }
        if let Some(Offer { id, video, opaque }) = message.offer {
            let media = if video { CallMediaType::Video } else { CallMediaType::Audio };
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
        } else if let Some(Opaque { id, opaque }) = message.answer {
            let Ok(answer) = signaling::Answer::new(opaque) else { return };
            let received =
                signaling::ReceivedAnswer { answer, sender_device_id: device, sender_identity_key: peer_key, receiver_identity_key: my_key };
            self.manager.received_answer(peer, CallId::new(id), received);
        } else if let Some(id) = message.ice.first().map(|i| i.id) {
            let candidates = message.ice.into_iter().map(|i| signaling::IceCandidate::new(i.opaque)).collect();
            let received = signaling::ReceivedIce { ice: signaling::Ice { candidates }, sender_device_id: device };
            self.manager.received_ice(peer, CallId::new(id), received);
        } else if let Some(Hangup { id, kind, device_id }) = message.hangup {
            let Some(kind) = signaling::HangupType::from_i32(kind) else { return };
            let hangup = signaling::Hangup::from_type_and_device_id(kind, device_id.unwrap_or_default());
            self.manager.received_hangup(peer, CallId::new(id), signaling::ReceivedHangup { hangup, sender_device_id: device });
        } else if let Some(id) = message.busy {
            self.manager.received_busy(peer, CallId::new(id), signaling::ReceivedBusy { sender_device_id: device });
        }
    }
}

/// The system's default microphone and speaker, as calling apps use them.
fn use_default_devices(factory: &mut PeerConnectionFactory) {
    if let Ok(devices) = factory.get_audio_playout_devices()
        && let Some(device) = devices.default.default_for_voip()
        && let Err(e) = factory.set_audio_playout_device_by_id(&device.unique_id, 0)
    {
        eprintln!("merel-signal-calls: no speaker: {e}");
    }
    if let Ok(devices) = factory.get_audio_recording_devices()
        && let Some(device) = devices.default.default_for_voip()
        && let Err(e) = factory.set_audio_recording_device_by_id(&device.unique_id, 0)
    {
        eprintln!("merel-signal-calls: no microphone: {e}");
    }
}

/// The message to send for what RingRTC says.
fn to_wire(call_id: u64, receiver: Option<DeviceId>, message: signaling::Message) -> Signalling {
    let id = call_id;
    let mut out = Signalling { destination_device_id: receiver, ..Default::default() };
    match message {
        signaling::Message::Offer(offer) => {
            out.offer = Some(Offer { id, video: offer.call_media_type == CallMediaType::Video, opaque: offer.opaque });
        }
        signaling::Message::Answer(answer) => out.answer = Some(Opaque { id, opaque: answer.opaque }),
        signaling::Message::Ice(ice) => out.ice = ice.candidates.into_iter().map(|c| Opaque { id, opaque: c.opaque }).collect(),
        signaling::Message::Hangup(hangup) => {
            let (kind, device_id) = hangup.to_type_and_device_id();
            out.hangup = Some(Hangup { id, kind: kind as i32, device_id });
        }
        signaling::Message::Busy => out.busy = Some(id),
    }
    out
}

/// RingRTC's callbacks, passed on to merel-signal.
struct Tell(Out);

impl SignalingSender for Tell {
    fn send_signaling(
        &self,
        recipient: &str,
        call_id: CallId,
        receiver: Option<DeviceId>,
        message: signaling::Message,
    ) -> ringrtc::common::Result<()> {
        let call_id = call_id.as_u64();
        self.0.say(&FromCalls::Send { to: recipient.to_string(), call_id, message: to_wire(call_id, receiver, message) });
        Ok(())
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

impl CallStateHandler for Tell {
    fn handle_call_state(&self, peer: &str, call_id: CallId, state: CallState) -> ringrtc::common::Result<()> {
        use ringrtc::common::CallEndReason as Why;
        let call_id = call_id.as_u64();
        let phase = match state {
            CallState::Incoming(_) => {
                self.0.say(&FromCalls::Proceed { call_id });
                Phase::Incoming
            }
            CallState::Outgoing(_) => {
                self.0.say(&FromCalls::Proceed { call_id });
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
        self.0.say(&FromCalls::State { peer: peer.to_string(), call_id, state: phase });
        Ok(())
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

impl GroupUpdateHandler for Tell {
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
        let message = to_wire(7, Some(2), signaling::Message::Hangup(signaling::Hangup::AcceptedOnAnotherDevice(3)));
        assert_eq!(message.hangup, Some(Hangup { id: 7, kind: 1, device_id: Some(3) }));
        assert_eq!(message.destination_device_id, Some(2));
    }

    #[test]
    fn ice_goes_out_as_one_update_per_candidate() {
        let ice = signaling::Ice { candidates: vec![signaling::IceCandidate::new(vec![1]), signaling::IceCandidate::new(vec![2])] };
        let message = to_wire(9, None, signaling::Message::Ice(ice));
        assert_eq!(message.ice.len(), 2);
        assert!(message.ice.iter().all(|u| u.id == 9));
    }
}
