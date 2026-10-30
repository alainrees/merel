//! Voice messages, as Signal has them: playing the ones that come in (AAC from the phones, MP3
//! from Signal Desktop) and recording new ones as MP3, which every Signal app plays.
//!
//! Sound is kept as one channel of `f32` samples between -1 and 1; the devices get or give it
//! at their own rate and channel count.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Most of a voice message Merel decodes or records: Signal's own limit is far above what
/// anyone speaks in one go.
pub const MAX_LENGTH: Duration = Duration::from_secs(30 * 60);

/// A piece of sound: one channel, `rate` samples per second.
#[derive(Clone, Debug)]
pub struct Clip {
    pub samples: Arc<[f32]>,
    pub rate: u32,
}

impl Clip {
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.rate.max(1)))
    }
}

/// The sound in `bytes`, a voice message of type `mime` (which only helps to guess).
pub fn decode(bytes: &[u8], mime: &str) -> Result<Clip, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.mime_type(mime);
    let probed =
        symphonia::default::get_probe().format(&hint, source, &FormatOptions::default(), &MetadataOptions::default()).map_err(|e| e.to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no sound in it")?;
    let id = track.id;
    let rate = track.codec_params.sample_rate.ok_or("no sample rate")?;
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
    let most = MAX_LENGTH.as_secs() as usize * rate as usize;
    let mut samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged packet is skipped, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        samples.extend(buffer.samples().chunks(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32));
        if samples.len() > most {
            samples.truncate(most);
            break;
        }
    }
    if samples.is_empty() {
        return Err("no sound in it".into());
    }
    Ok(Clip { samples: samples.into(), rate })
}

/// `clip` as MP3, mono, as small as speech allows.
pub fn mp3(clip: &Clip) -> Result<Vec<u8>, String> {
    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, MonoPcm, Quality};

    let mut builder = Builder::new().ok_or("no MP3 encoder")?;
    builder.set_num_channels(1).map_err(|e| e.to_string())?;
    builder.set_sample_rate(clip.rate).map_err(|e| e.to_string())?;
    builder.set_brate(Bitrate::Kbps64).map_err(|e| e.to_string())?;
    builder.set_quality(Quality::Good).map_err(|e| e.to_string())?;
    let mut encoder = builder.build().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for chunk in clip.samples.chunks(8192) {
        out.reserve(mp3lame_encoder::max_required_buffer_size(chunk.len()));
        encoder.encode_to_vec(MonoPcm(chunk), &mut out).map_err(|e| e.to_string())?;
    }
    out.reserve(7200);
    encoder.flush_to_vec::<FlushNoGap>(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// A clip playing on the speakers; dropping it stops.
pub struct Playback {
    _stream: cpal::Stream,
    /// How far it is, in samples of the clip.
    at: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
    length: u64,
}

impl Playback {
    /// How far it is, from 0 to 1.
    pub fn progress(&self) -> f32 {
        (self.at.load(Ordering::Relaxed) as f64 / self.length.max(1) as f64).min(1.0) as f32
    }

    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
}

/// Plays `clip` on the computer's speakers, from `from` (0 to 1).
pub fn play(clip: &Clip, from: f32) -> Result<Playback, String> {
    let device = cpal::default_host().default_output_device().ok_or("no speakers")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?.config();
    let channels = usize::from(config.channels.max(1));
    let step = f64::from(clip.rate) / f64::from(config.sample_rate.max(1));
    let length = clip.samples.len() as u64;
    let at = Arc::new(AtomicU64::new((f64::from(from.clamp(0.0, 1.0)) * length as f64) as u64));
    let done = Arc::new(AtomicBool::new(false));
    let samples = clip.samples.clone();
    let (at_out, done_out) = (at.clone(), done.clone());
    let mut pos = at.load(Ordering::Relaxed) as f64;
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |out, _| {
                for frame in out.chunks_mut(channels) {
                    let i = pos as usize;
                    let value = match (samples.get(i), samples.get(i + 1)) {
                        (Some(a), Some(b)) => a + (b - a) * (pos.fract() as f32),
                        (Some(a), None) => *a,
                        _ => {
                            done_out.store(true, Ordering::Relaxed);
                            0.0
                        }
                    };
                    frame.fill(value);
                    pos += step;
                }
                at_out.store(pos as u64, Ordering::Relaxed);
            },
            |e| log::warn!("speakers: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Playback { _stream: stream, at, done, length })
}

/// The microphone recording; [`Recording::finish`] gives what it heard.
pub struct Recording {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    /// How loud it is now, from 0 to 1, for a meter.
    level: Arc<AtomicU64>,
    rate: u32,
}

impl Recording {
    /// How long it has been recording.
    pub fn elapsed(&self) -> Duration {
        let n = self.samples.lock().unwrap_or_else(PoisonError::into_inner).len();
        Duration::from_secs_f64(n as f64 / f64::from(self.rate.max(1)))
    }

    /// How loud the microphone is now, from 0 to 1.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed) as u32)
    }

    /// Stops and gives what it heard.
    pub fn finish(self) -> Clip {
        let _ = self.stream.pause();
        drop(self.stream);
        let samples = std::mem::take(&mut *self.samples.lock().unwrap_or_else(PoisonError::into_inner));
        Clip { samples: samples.into(), rate: self.rate }
    }
}

/// Starts recording from the computer's microphone.
pub fn record() -> Result<Recording, String> {
    let device = cpal::default_host().default_input_device().ok_or("no microphone")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?.config();
    let channels = usize::from(config.channels.max(1));
    let rate = config.sample_rate.max(1);
    let most = MAX_LENGTH.as_secs() as usize * rate as usize;
    let samples = Arc::new(Mutex::new(Vec::new()));
    let level = Arc::new(AtomicU64::new(0));
    let (into, level_in) = (samples.clone(), level.clone());
    let stream = device
        .build_input_stream::<f32, _, _>(
            config,
            move |data, _| {
                let mut kept = into.lock().unwrap_or_else(PoisonError::into_inner);
                let mut peak = 0.0f32;
                for frame in data.chunks(channels) {
                    let value = frame.iter().sum::<f32>() / frame.len() as f32;
                    peak = peak.max(value.abs());
                    if kept.len() < most {
                        kept.push(value);
                    }
                }
                level_in.store(u64::from(peak.min(1.0).to_bits()), Ordering::Relaxed);
            },
            |e| log::warn!("microphone: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Recording { stream, samples, level, rate })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_recorded_comes_back_as_mp3_of_the_same_length() {
        let rate = 44_100;
        let tone: Vec<f32> = (0..rate * 2).map(|n| (n as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5).collect();
        let clip = Clip { samples: tone.into(), rate };
        let bytes = mp3(&clip).unwrap();
        assert!(bytes.len() > 1000 && bytes.len() < 40_000, "{} bytes", bytes.len());
        let back = decode(&bytes, "audio/mpeg").unwrap();
        assert_eq!(back.rate, rate);
        let seconds = back.duration().as_secs_f32();
        assert!((1.9..2.2).contains(&seconds), "{seconds} s");
        // Still a tone, not silence.
        assert!(back.samples.iter().any(|s| s.abs() > 0.3));
    }

    #[test]
    fn rubbish_is_no_sound() {
        assert!(decode(b"not a sound at all", "audio/aac").is_err());
    }
}
