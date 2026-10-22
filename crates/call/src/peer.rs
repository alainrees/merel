//! The WebRTC side of [`crate::Media`]: one connection per person per room, each with an Opus
//! track out and whatever comes in decoded into the mixer.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use audiopus::coder::{Decoder, Encoder};
use audiopus::{Application, Channels, MutSignals, SampleRate, packet::Packet};
use rtc::media::Sample;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::media_engine::MIME_TYPE_OPUS;
use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind};
use tokio::sync::{Mutex, mpsc};
use webrtc::media_stream::track_local::{TrackLocal, static_sample::TrackLocalStaticSample};
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder, RTCIceGatheringState, RTCIceServer,
    RTCPeerConnectionState, RTCSessionDescription,
};

use crate::audio::{self, Mixer};
use crate::{Command, MediaEvent};

/// The payload type Opus gets in the offer; the answer may pick another.
const OPUS_PT: u8 = 111;
/// How long gathering addresses may take before the description goes out with what there is.
const GATHERING: Duration = Duration::from_secs(5);
/// Every how many frames a level goes out (five frames: ten times a second).
const LEVEL_EVERY: u32 = 5;

/// One connection's key: the room and the other person.
type Key = (String, String);

/// One connection.
pub(crate) struct Peer {
    pc: Arc<dyn PeerConnection>,
    track: Arc<TrackLocalStaticSample>,
    ssrc: u32,
    /// The payload type both sides agreed on; none until the answer is in.
    pt: Option<u8>,
}

type Peers = Arc<Mutex<HashMap<Key, Peer>>>;

/// Events go back on a plain channel: the app reads them from its own thread.
type Events = std::sync::mpsc::Sender<MediaEvent>;

fn opus() -> RTCRtpCodec {
    RTCRtpCodec {
        mime_type: MIME_TYPE_OPUS.into(),
        clock_rate: audio::RATE,
        channels: 2,
        sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
        rtcp_feedback: Vec::new(),
    }
}

/// Runs until [`Command::Stop`]: takes commands and microphone frames, sends each frame to
/// everyone connected.
pub(crate) async fn run(
    ice: Vec<String>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut frames: mpsc::UnboundedReceiver<Vec<i16>>,
    events: Events,
    mixer: Arc<Mixer>,
) {
    let peers: Peers = Arc::default();
    let ice = Arc::new(ice);
    let mut encoder = match Encoder::new(SampleRate::Hz48000, Channels::Mono, Application::Voip) {
        Ok(e) => Some(e),
        Err(e) => {
            log::warn!("no Opus encoder: {e}");
            None
        }
    };
    let mut packet = vec![0u8; 4000];
    let mut count = 0u32;
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                if matches!(command, Command::Stop) {
                    break;
                }
                let (peers, ice, events, mixer) = (peers.clone(), ice.clone(), events.clone(), mixer.clone());
                tokio::spawn(async move { handle(command, &peers, &ice, &events, &mixer).await });
            }
            frame = frames.recv() => {
                let Some(frame) = frame else { continue };
                count = count.wrapping_add(1);
                if count.is_multiple_of(LEVEL_EVERY) {
                    let _ = events.send(MediaEvent::Level { peer: String::new(), level: audio::level(&frame) });
                }
                let Some(encoder) = encoder.as_mut() else { continue };
                let n = match encoder.encode(&frame, &mut packet) {
                    Ok(n) => n,
                    Err(e) => {
                        log::warn!("Opus encoding failed: {e}");
                        continue;
                    }
                };
                let data = bytes::Bytes::copy_from_slice(packet.get(..n).unwrap_or_default());
                let senders: Vec<_> = peers.lock().await.values().filter_map(|p| Some((p.track.clone(), p.ssrc, p.pt?))).collect();
                for (track, ssrc, pt) in senders {
                    let sample = Sample { data: data.clone(), duration: Duration::from_millis(20), ..Sample::new(Instant::now()) };
                    if let Err(e) = track.sample_writer(ssrc, pt).write_sample(&sample).await {
                        log::debug!("sending sound failed: {e}");
                    }
                }
            }
        }
    }
    let all: Vec<_> = peers.lock().await.drain().map(|(_, p)| p.pc).collect();
    for pc in all {
        let _ = pc.close().await;
    }
}

async fn handle(command: Command, peers: &Peers, ice: &[String], events: &Events, mixer: &Arc<Mixer>) {
    let (room, peer, result) = match command {
        Command::Offer { room, peer } => {
            let r = offer(&room, &peer, peers, ice, events, mixer).await;
            (room, peer, r)
        }
        Command::Answer { room, peer, sdp } => {
            let r = answer(&room, &peer, sdp, peers, ice, events, mixer).await;
            (room, peer, r)
        }
        Command::Accept { room, peer, sdp } => {
            let r = accept(&room, &peer, sdp, peers).await;
            (room, peer, r)
        }
        Command::Close { room, peer } => {
            close(&room, &peer, peers, mixer).await;
            return;
        }
        Command::Stop => return,
    };
    if let Err(what) = result {
        log::warn!("call {room} with {peer}: {what}");
        close(&room, &peer, peers, mixer).await;
        let _ = events.send(MediaEvent::Failed { room, peer, what });
    }
}

/// A new connection with `peer`, with the sound track added, kept under its key. An old one
/// under the same key is closed first.
async fn connect(
    room: &str,
    peer: &str,
    peers: &Peers,
    ice: &[String],
    events: &Events,
    mixer: &Arc<Mixer>,
) -> Result<(Arc<dyn PeerConnection>, mpsc::UnboundedReceiver<()>), String> {
    close(room, peer, peers, mixer).await;
    let mut engine = MediaEngine::default();
    engine
        .register_codec(RTCRtpCodecParameters { rtp_codec: opus(), payload_type: OPUS_PT }, RtpCodecKind::Audio)
        .map_err(|e| format!("codec: {e}"))?;
    let mut config = RTCConfigurationBuilder::new();
    if !ice.is_empty() {
        config = config.with_ice_servers(vec![RTCIceServer { urls: ice.to_vec(), ..Default::default() }]);
    }
    let (gathered, gathered_rx) = mpsc::unbounded_channel();
    let handler = Handler { room: room.into(), peer: peer.into(), events: events.clone(), mixer: mixer.clone(), gathered };
    let pc = PeerConnectionBuilder::new()
        .with_configuration(config.build())
        .with_media_engine(engine)
        .with_handler(Arc::new(handler))
        .with_udp_addrs(vec!["0.0.0.0:0"])
        .build()
        .await
        .map_err(|e| format!("connection: {e}"))?;
    let pc: Arc<dyn PeerConnection> = Arc::new(pc);
    let ssrc = next_ssrc();
    let track = Arc::new(
        TrackLocalStaticSample::new(
            Instant::now(),
            MediaStreamTrack::new(
                "merel".into(),
                "audio".into(),
                "audio".into(),
                RtpCodecKind::Audio,
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters { ssrc: Some(ssrc), ..Default::default() },
                    codec: opus(),
                    ..Default::default()
                }],
            ),
        )
        .map_err(|e| format!("track: {e}"))?,
    );
    peers.lock().await.insert((room.into(), peer.into()), Peer { pc: pc.clone(), track: track.clone(), ssrc, pt: None });
    Ok((pc, gathered_rx))
}

async fn add_track(room: &str, peer: &str, pc: &Arc<dyn PeerConnection>, peers: &Peers) -> Result<(), String> {
    let track = peers.lock().await.get(&(room.into(), peer.into())).map(|p| p.track.clone()).ok_or("closed")?;
    pc.add_track(track as Arc<dyn TrackLocal>).await.map_err(|e| format!("add track: {e}"))?;
    Ok(())
}

/// The local description once its addresses are gathered (no trickle: the steps go by mail).
async fn described(pc: &Arc<dyn PeerConnection>, mut gathered: mpsc::UnboundedReceiver<()>) -> Result<String, String> {
    let _ = tokio::time::timeout(GATHERING, gathered.recv()).await;
    pc.local_description().await.map(|d| d.sdp).ok_or_else(|| "no local description".into())
}

async fn offer(room: &str, peer: &str, peers: &Peers, ice: &[String], events: &Events, mixer: &Arc<Mixer>) -> Result<(), String> {
    let (pc, gathered) = connect(room, peer, peers, ice, events, mixer).await?;
    add_track(room, peer, &pc, peers).await?;
    let offer = pc.create_offer(None).await.map_err(|e| format!("offer: {e}"))?;
    pc.set_local_description(offer).await.map_err(|e| format!("offer: {e}"))?;
    let sdp = described(&pc, gathered).await?;
    let _ = events.send(MediaEvent::Offer { room: room.into(), peer: peer.into(), sdp });
    Ok(())
}

async fn answer(room: &str, peer: &str, sdp: String, peers: &Peers, ice: &[String], events: &Events, mixer: &Arc<Mixer>) -> Result<(), String> {
    let (pc, gathered) = connect(room, peer, peers, ice, events, mixer).await?;
    let offer = RTCSessionDescription::offer(sdp).map_err(|e| format!("their offer: {e}"))?;
    pc.set_remote_description(offer).await.map_err(|e| format!("their offer: {e}"))?;
    add_track(room, peer, &pc, peers).await?;
    let answer = pc.create_answer(None).await.map_err(|e| format!("answer: {e}"))?;
    pc.set_local_description(answer).await.map_err(|e| format!("answer: {e}"))?;
    agreed(room, peer, &pc, peers).await?;
    let sdp = described(&pc, gathered).await?;
    let _ = events.send(MediaEvent::Answer { room: room.into(), peer: peer.into(), sdp });
    Ok(())
}

async fn accept(room: &str, peer: &str, sdp: String, peers: &Peers) -> Result<(), String> {
    let pc = peers.lock().await.get(&(room.into(), peer.into())).map(|p| p.pc.clone()).ok_or("no offer of ours")?;
    let answer = RTCSessionDescription::answer(sdp).map_err(|e| format!("their answer: {e}"))?;
    pc.set_remote_description(answer).await.map_err(|e| format!("their answer: {e}"))?;
    agreed(room, peer, &pc, peers).await
}

/// Notes the payload type both sides settled on, so sound can go out.
async fn agreed(room: &str, peer: &str, pc: &Arc<dyn PeerConnection>, peers: &Peers) -> Result<(), String> {
    let sender = pc.get_senders().await.into_iter().next().ok_or("no sender")?;
    let parameters = sender.get_parameters().await.map_err(|e| format!("parameters: {e}"))?;
    let pt = parameters.rtp_parameters.codecs.first().map(|c| c.payload_type).ok_or("no codec agreed")?;
    if let Some(p) = peers.lock().await.get_mut(&(room.into(), peer.into())) {
        p.pt = Some(pt);
    }
    Ok(())
}

async fn close(room: &str, peer: &str, peers: &Peers, mixer: &Mixer) {
    let old = peers.lock().await.remove(&(room.into(), peer.into()));
    if let Some(old) = old {
        let _ = old.pc.close().await;
        mixer.remove(peer);
    }
}

/// A fresh synchronisation source per track, so two tracks never share one.
fn next_ssrc() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1);
    seed.wrapping_mul(2_654_435_761).wrapping_add(NEXT.fetch_add(1, Ordering::Relaxed)) | 1
}

struct Handler {
    room: String,
    peer: String,
    events: Events,
    mixer: Arc<Mixer>,
    gathered: mpsc::UnboundedSender<()>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gathered.send(());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let connected = match state {
            RTCPeerConnectionState::Connected => true,
            RTCPeerConnectionState::Disconnected | RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => false,
            _ => return,
        };
        let _ = self.events.send(MediaEvent::Connected { room: self.room.clone(), peer: self.peer.clone(), connected });
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let (peer, events, mixer) = (self.peer.clone(), self.events.clone(), self.mixer.clone());
        tokio::spawn(async move {
            let mut decoder = match Decoder::new(SampleRate::Hz48000, Channels::Mono) {
                Ok(d) => d,
                Err(e) => {
                    log::warn!("no Opus decoder: {e}");
                    return;
                }
            };
            // Room for the longest Opus frame, 120 ms.
            let mut pcm = vec![0i16; 5760];
            let mut count = 0u32;
            while let Some(event) = track.poll().await {
                let TrackRemoteEvent::OnRtpPacket(p) = event else { continue };
                let Ok(packet) = Packet::try_from(&p.payload[..]) else { continue };
                let Ok(signals) = MutSignals::try_from(&mut pcm[..]) else { continue };
                let Ok(n) = decoder.decode(Some(packet), signals, false) else { continue };
                let got = pcm.get(..n).unwrap_or_default();
                mixer.push(&peer, got);
                count = count.wrapping_add(1);
                if count.is_multiple_of(LEVEL_EVERY) {
                    let _ = events.send(MediaEvent::Level { peer: peer.clone(), level: audio::level(got) });
                }
            }
        });
    }
}
