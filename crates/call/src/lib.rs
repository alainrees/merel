//! Merel's own meeting room: sound goes straight between the computers over WebRTC, encrypted
//! end to end with DTLS-SRTP. The connection steps (offers and answers) travel as encrypted
//! chat messages through the engine; this crate makes them and plays what comes in.
//!
//! [`Media`] runs WebRTC on a thread of its own and is driven by plain calls: `offer`,
//! `answer`, `accept`, `close`. What happens comes back from [`Media::events`]. The microphone
//! feeds [`Media::microphone`]; the speakers pull from [`Media::mixer`]. [`devices`] connects
//! both to the computer's own sound devices.

pub mod audio;
pub mod devices;
mod peer;

use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

pub use audio::Mixer;

/// The STUN server that tells each computer its address on the internet, so two computers
/// behind home routers can reach each other. It only sees that address, never the sound.
/// Nextcloud runs it for Nextcloud Talk, free.
pub const DEFAULT_STUN: &str = "stun:stun.nextcloud.com:443";

/// What [`Media`] reports.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaEvent {
    /// The offer for `peer`, to send to them.
    Offer { room: String, peer: String, sdp: String },
    /// The answer to `peer`'s offer, to send to them.
    Answer { room: String, peer: String, sdp: String },
    /// The connection with `peer` came up, or went down.
    Connected { room: String, peer: String, connected: bool },
    /// How loud `peer` is now, 0.0 to 1.0; the account itself is the empty peer.
    Level { peer: String, level: f32 },
    /// Something went wrong with `peer`, in English for the log.
    Failed { room: String, peer: String, what: String },
}

enum Command {
    Offer { room: String, peer: String },
    Answer { room: String, peer: String, sdp: String },
    Accept { room: String, peer: String, sdp: String },
    Close { room: String, peer: String },
    Stop,
}

/// Feeds the microphone's sound into the meeting; cheap to clone, safe to use from a device
/// callback.
#[derive(Clone, Debug)]
pub struct Microphone {
    frames: mpsc::UnboundedSender<Vec<i16>>,
    muted: Arc<std::sync::atomic::AtomicBool>,
}

impl Microphone {
    /// Hands over one frame of [`audio::FRAME`] samples, mono at [`audio::RATE`].
    pub fn send(&self, frame: Vec<i16>) {
        if !self.muted.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = self.frames.send(frame);
        }
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_muted(&self) -> bool {
        self.muted.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// WebRTC for one computer: every connection of every room it is in.
pub struct Media {
    commands: mpsc::UnboundedSender<Command>,
    events: Mutex<std::sync::mpsc::Receiver<MediaEvent>>,
    microphone: Microphone,
    mixer: Arc<Mixer>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Media {
    /// Starts WebRTC on a thread of its own. `ice` lists the STUN (and later TURN) servers.
    pub fn start(ice: Vec<String>) -> std::io::Result<Media> {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (event_tx, events) = std::sync::mpsc::channel();
        let (frames, frame_rx) = mpsc::unbounded_channel();
        let mixer = Arc::new(Mixer::default());
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("merel-call").build()?;
        let shared = mixer.clone();
        let thread = std::thread::Builder::new()
            .name("merel-call".into())
            .spawn(move || runtime.block_on(peer::run(ice, command_rx, frame_rx, event_tx, shared)))?;
        Ok(Media {
            commands,
            events: Mutex::new(events),
            microphone: Microphone { frames, muted: Arc::new(std::sync::atomic::AtomicBool::new(false)) },
            mixer,
            thread: Some(thread),
        })
    }

    /// Makes an offer for `peer`; it comes back as [`MediaEvent::Offer`].
    pub fn offer(&self, room: &str, peer: &str) {
        let _ = self.commands.send(Command::Offer { room: room.into(), peer: peer.into() });
    }

    /// Answers `peer`'s offer; the answer comes back as [`MediaEvent::Answer`].
    pub fn answer(&self, room: &str, peer: &str, sdp: String) {
        let _ = self.commands.send(Command::Answer { room: room.into(), peer: peer.into(), sdp });
    }

    /// Takes `peer`'s answer to this computer's offer.
    pub fn accept(&self, room: &str, peer: &str, sdp: String) {
        let _ = self.commands.send(Command::Accept { room: room.into(), peer: peer.into(), sdp });
    }

    /// Closes the connection with `peer`.
    pub fn close(&self, room: &str, peer: &str) {
        let _ = self.commands.send(Command::Close { room: room.into(), peer: peer.into() });
    }

    /// What happened since the last call, in order.
    pub fn events(&self) -> Vec<MediaEvent> {
        self.events.lock().map(|rx| rx.try_iter().collect()).unwrap_or_default()
    }

    pub fn microphone(&self) -> &Microphone {
        &self.microphone
    }

    /// Everyone else's sound, for the speakers.
    pub fn mixer(&self) -> &Arc<Mixer> {
        &self.mixer
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The DTLS certificate fingerprints a session description names (`a=fingerprint:`), upper
/// case.
pub fn fingerprints(sdp: &str) -> Vec<String> {
    sdp.lines().filter_map(|l| l.trim().strip_prefix("a=fingerprint:")).map(|f| f.trim().to_ascii_uppercase()).collect()
}

/// The safety code of a connection: twelve digits from both sides' DTLS fingerprints, the same
/// on both computers whoever offered. Read aloud and compared, it shows that nothing sits in
/// between.
pub fn safety_code(local_sdp: &str, remote_sdp: &str) -> String {
    let mut prints = fingerprints(local_sdp);
    prints.extend(fingerprints(remote_sdp));
    prints.sort();
    let digest = Sha256::digest(prints.join("\n").as_bytes());
    let n = digest.iter().take(8).fold(0u64, |acc, b| (acc << 8) | u64::from(*b)) % 1_000_000_000_000;
    let s = format!("{n:012}");
    format!("{} · {} · {}", &s[..4], &s[4..8], &s[8..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_safety_code_is_the_same_on_both_sides() {
        let a = "v=0\r\na=fingerprint:sha-256 AA:BB\r\n";
        let b = "v=0\r\na=fingerprint:sha-256 cc:dd\r\n";
        assert_eq!(safety_code(a, b), safety_code(b, a));
        assert_ne!(safety_code(a, b), safety_code(a, a));
        assert_eq!(safety_code(a, b).len(), 4 + 3 + 4 + 3 + 4 + 2 * ("·".len() - 1));
    }

    /// Two computers in one process: an offer, an answer, and a tone from one arrives at the
    /// other's speakers.
    #[test]
    fn sound_goes_from_one_computer_to_the_other() {
        let a = Media::start(Vec::new()).unwrap();
        let b = Media::start(Vec::new()).unwrap();
        let wait = |m: &Media, want: &dyn Fn(&MediaEvent) -> bool| {
            for _ in 0..200 {
                if let Some(e) = m.events().into_iter().find(|e| want(e)) {
                    return e;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            panic!("no event");
        };
        a.offer("R1", "b@x");
        let MediaEvent::Offer { sdp, .. } = wait(&a, &|e| matches!(e, MediaEvent::Offer { .. })) else { unreachable!() };
        b.answer("R1", "a@x", sdp);
        let MediaEvent::Answer { sdp, .. } = wait(&b, &|e| matches!(e, MediaEvent::Answer { .. })) else { unreachable!() };
        a.accept("R1", "b@x", sdp);
        wait(&a, &|e| matches!(e, MediaEvent::Connected { connected: true, .. }));

        // A tone of 440 Hz from A's microphone.
        let mut t = 0f64;
        let mut heard = 0.0f32;
        for _ in 0..150 {
            let frame: Vec<i16> = (0..audio::FRAME)
                .map(|_| {
                    t += 1.0;
                    ((t * 440.0 * std::f64::consts::TAU / 48_000.0).sin() * 8_000.0) as i16
                })
                .collect();
            a.microphone().send(frame);
            std::thread::sleep(std::time::Duration::from_millis(20));
            let mut out = vec![0i16; audio::FRAME];
            b.mixer().pull(&mut out);
            heard = heard.max(audio::level(&out));
            if heard > 0.05 {
                break;
            }
        }
        assert!(heard > 0.05, "B heard {heard}");
        assert!(b.events().iter().chain(&[]).all(|e| !matches!(e, MediaEvent::Failed { .. })));
        a.close("R1", "b@x");
    }
}
