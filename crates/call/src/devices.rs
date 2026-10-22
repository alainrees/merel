//! The computer's own microphone and speakers, connected to a [`Media`](crate::Media).

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::Microphone;
use crate::audio::{Framer, Mixer, Player};

/// The open sound devices; dropping them stops both.
pub struct Devices {
    _input: Option<cpal::Stream>,
    _output: Option<cpal::Stream>,
    /// The names of the microphone and speakers in use, for the meeting room.
    pub input_name: Option<String>,
    pub output_name: Option<String>,
}

/// Opens the default microphone into `microphone` and the default speakers on `mixer`. Either
/// may be missing (a computer without a microphone can still listen); what failed is logged.
pub fn open(microphone: Microphone, mixer: Arc<Mixer>) -> Devices {
    let host = cpal::default_host();
    let mut devices = Devices { _input: None, _output: None, input_name: None, output_name: None };
    if let Some(device) = host.default_input_device() {
        match open_input(&device, microphone) {
            Ok(s) => {
                devices.input_name = Some(device.to_string());
                devices._input = Some(s);
            }
            Err(e) => log::warn!("microphone: {e}"),
        }
    }
    if let Some(device) = host.default_output_device() {
        match open_output(&device, mixer) {
            Ok(s) => {
                devices.output_name = Some(device.to_string());
                devices._output = Some(s);
            }
            Err(e) => log::warn!("speakers: {e}"),
        }
    }
    devices
}

fn open_input(device: &cpal::Device, microphone: Microphone) -> Result<cpal::Stream, String> {
    let config = device.default_input_config().map_err(|e| e.to_string())?.config();
    let mut framer = Framer::new(config.sample_rate, config.channels);
    let stream = device
        .build_input_stream::<f32, _, _>(
            config,
            move |data, _| framer.push(data, |frame| microphone.send(frame)),
            |e| log::warn!("microphone: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn open_output(device: &cpal::Device, mixer: Arc<Mixer>) -> Result<cpal::Stream, String> {
    let config = device.default_output_config().map_err(|e| e.to_string())?.config();
    let mut player = Player::new(config.sample_rate, config.channels);
    let stream = device
        .build_output_stream::<f32, _, _>(config, move |out, _| player.fill(&mixer, out), |e| log::warn!("speakers: {e}"), None)
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}
