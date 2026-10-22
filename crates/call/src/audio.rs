//! Sound between the devices and the codec: what the microphone gives is cut into frames of
//! 20 ms, mono, 48 kHz (what Opus sends); what comes in from the others is mixed into one
//! stream for the speakers.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// The rate Opus runs at in WebRTC.
pub const RATE: u32 = 48_000;
/// Samples in one frame of 20 ms.
pub const FRAME: usize = 960;
/// Most sound kept waiting per person before the oldest is dropped: 200 ms, so a late burst
/// does not pile up as delay.
const MAX_QUEUE: usize = FRAME * 10;

/// Turns sound at any rate and channel count into mono frames at [`RATE`]: channels are
/// averaged, the rate is converted by straight-line interpolation (plenty for speech).
#[derive(Debug)]
pub struct Framer {
    rate: u32,
    channels: usize,
    /// Where between input samples the next output sample falls.
    pos: f64,
    last: f32,
    pending: Vec<i16>,
}

impl Framer {
    pub fn new(rate: u32, channels: u16) -> Framer {
        Framer { rate: rate.max(1), channels: usize::from(channels.max(1)), pos: 0.0, last: 0.0, pending: Vec::with_capacity(FRAME * 2) }
    }

    /// Takes interleaved samples from the device and hands over every whole frame.
    pub fn push(&mut self, data: &[f32], mut frame: impl FnMut(Vec<i16>)) {
        let step = f64::from(self.rate) / f64::from(RATE);
        for chunk in data.chunks(self.channels) {
            let sample = chunk.iter().sum::<f32>() / chunk.len() as f32;
            // Every output sample between the last input sample and this one.
            while self.pos < 1.0 {
                let v = self.last + (sample - self.last) * self.pos as f32;
                self.pending.push(to_i16(v));
                self.pos += step;
            }
            self.pos -= 1.0;
            self.last = sample;
            if self.pending.len() >= FRAME {
                let rest = self.pending.split_off(FRAME);
                frame(std::mem::replace(&mut self.pending, rest));
            }
        }
    }
}

fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

/// The loudness of a frame, 0.0 to 1.0 (root mean square).
pub fn level(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let sum: f64 = pcm.iter().map(|s| f64::from(*s).powi(2)).sum();
    ((sum / pcm.len() as f64).sqrt() / f64::from(i16::MAX)) as f32
}

/// The sound of everyone else, per person, mixed when the speakers ask for it.
#[derive(Debug, Default)]
pub struct Mixer {
    queues: Mutex<HashMap<String, VecDeque<i16>>>,
}

impl Mixer {
    /// Adds decoded sound from `peer`.
    pub fn push(&self, peer: &str, pcm: &[i16]) {
        let Ok(mut queues) = self.queues.lock() else { return };
        let q = queues.entry(peer.to_string()).or_default();
        q.extend(pcm);
        let over = q.len().saturating_sub(MAX_QUEUE);
        q.drain(..over);
    }

    /// Forgets `peer`, who left.
    pub fn remove(&self, peer: &str) {
        if let Ok(mut queues) = self.queues.lock() {
            queues.remove(peer);
        }
    }

    /// Fills `out` with everyone's sound added up, mono at [`RATE`]; silence where nothing
    /// waits.
    pub fn pull(&self, out: &mut [i16]) {
        out.fill(0);
        let Ok(mut queues) = self.queues.lock() else { return };
        let len = out.len();
        for q in queues.values_mut() {
            for (o, s) in out.iter_mut().zip(q.drain(..q.len().min(len))) {
                *o = o.saturating_add(s);
            }
        }
    }
}

/// Plays mono sound at [`RATE`] on a device with another rate and channel count.
#[derive(Debug)]
pub struct Player {
    rate: u32,
    channels: usize,
    pos: f64,
    buf: Vec<i16>,
    at: usize,
    last: f32,
}

impl Player {
    pub fn new(rate: u32, channels: u16) -> Player {
        Player { rate: rate.max(1), channels: usize::from(channels.max(1)), pos: 0.0, buf: Vec::new(), at: 0, last: 0.0 }
    }

    /// Fills the device's interleaved buffer `out` from `mixer`.
    pub fn fill(&mut self, mixer: &Mixer, out: &mut [f32]) {
        let step = f64::from(RATE) / f64::from(self.rate);
        for frame in out.chunks_mut(self.channels) {
            while self.pos >= 1.0 {
                if self.at >= self.buf.len() {
                    self.buf.resize(FRAME / 4, 0);
                    mixer.pull(&mut self.buf);
                    self.at = 0;
                }
                self.last = f32::from(self.buf[self.at]) / f32::from(i16::MAX);
                self.at += 1;
                self.pos -= 1.0;
            }
            frame.fill(self.last);
            self.pos += step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_sound_becomes_mono_frames_at_48_khz() {
        // One second of stereo at 44.1 kHz: about fifty frames of 960.
        let mut f = Framer::new(44_100, 2);
        let data: Vec<f32> = (0..44_100).flat_map(|i| [((i as f32) * 0.01).sin() * 0.5; 2]).collect();
        let mut frames = Vec::new();
        f.push(&data, |fr| frames.push(fr));
        assert!((49..=50).contains(&frames.len()), "{}", frames.len());
        assert!(frames.iter().all(|fr| fr.len() == FRAME));
        assert!(level(&frames[10]) > 0.2);
    }

    #[test]
    fn the_mixer_adds_people_up_and_drops_what_is_too_late() {
        let m = Mixer::default();
        m.push("a", &[100; 10]);
        m.push("b", &[50; 5]);
        let mut out = [0i16; 10];
        m.pull(&mut out);
        assert_eq!(out[..5], [150; 5]);
        assert_eq!(out[5..], [100; 5]);
        m.pull(&mut out);
        assert_eq!(out, [0; 10], "silence once it is played");
        m.push("a", &vec![1; MAX_QUEUE + 500]);
        assert_eq!(m.queues.lock().unwrap()["a"].len(), MAX_QUEUE);
        m.remove("a");
        assert!(!m.queues.lock().unwrap().contains_key("a"));
    }

    #[test]
    fn the_player_spreads_mono_over_the_device_channels() {
        let m = Mixer::default();
        m.push("a", &[i16::MAX; FRAME]);
        let mut p = Player::new(48_000, 2);
        let mut out = vec![0.0f32; 200];
        p.fill(&m, &mut out);
        assert!(out[10..].iter().all(|s| (*s - 1.0).abs() < 1e-3));
        assert_eq!(out[10], out[11], "both channels the same");
    }
}
