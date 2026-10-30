//! What `merel-signal` and `merel-signal-calls` say to each other, one JSON object per line.
//!
//! Calls run in a program of their own: Signal's calling library brings WebRTC with BoringSSL
//! built in, and that cannot be linked next to the OpenSSL the encrypted message store uses. So
//! `merel-signal` keeps the Signal account and carries the call messages; `merel-signal-calls`
//! runs RingRTC, plays and records the sound, and never sees the account or its store.

use serde::{Deserialize, Serialize};

/// From `merel-signal` to `merel-signal-calls`.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToCalls {
    /// Call `peer` (a service id) with this new call id, sound only.
    Call {
        peer: String,
        call_id: u64,
        local_device: u32,
    },
    Accept {
        call_id: u64,
    },
    /// End the call going on, or decline the one ringing.
    HangUp,
    Mute {
        on: bool,
    },
    /// Go on with a call that is set up, through these relays.
    Proceed {
        call_id: u64,
        relays: Vec<Relay>,
    },
    /// Whether the last message for this call went out.
    Sent {
        call_id: u64,
        ok: bool,
    },
    /// A call message came from `peer`'s device `device`. The keys are both accounts' public
    /// identity keys without their type byte; RingRTC mixes them into the call's keys.
    Received {
        peer: String,
        device: u32,
        my_device: u32,
        age_ms: u64,
        #[serde(with = "hex")]
        peer_key: Vec<u8>,
        #[serde(with = "hex")]
        my_key: Vec<u8>,
        message: Signalling,
    },
}

/// From `merel-signal-calls` to `merel-signal`.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromCalls {
    /// Calls can be made: sound works.
    Ready,
    /// Send this to `to` over Signal.
    Send {
        to: String,
        call_id: u64,
        message: Signalling,
    },
    /// A call is set up: it may go on once it has relays.
    Proceed {
        call_id: u64,
    },
    State {
        peer: String,
        call_id: u64,
        state: Phase,
    },
}

/// A call's phase as Merel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

impl Phase {
    pub fn is_over(self) -> bool {
        matches!(self, Phase::AnsweredElsewhere | Phase::Declined | Phase::Ended | Phase::Failed)
    }
}

/// Short-lived credentials for one of Signal's relays.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Relay {
    pub username: String,
    pub password: String,
    pub hostname: String,
    pub urls: Vec<String>,
    pub urls_with_ips: Vec<String>,
}

/// One call message, as Signal's `CallMessage` carries it.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct Signalling {
    #[serde(default)]
    pub destination_device_id: Option<u32>,
    #[serde(default)]
    pub offer: Option<Offer>,
    #[serde(default)]
    pub answer: Option<Opaque>,
    #[serde(default)]
    pub ice: Vec<Opaque>,
    #[serde(default)]
    pub hangup: Option<Hangup>,
    #[serde(default)]
    pub busy: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub id: u64,
    pub video: bool,
    #[serde(with = "hex")]
    pub opaque: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Opaque {
    pub id: u64,
    #[serde(with = "hex")]
    pub opaque: Vec<u8>,
}

/// `kind` as Signal numbers them: 0 normal, 1 accepted, 2 declined, 3 busy, 4 need permission.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Hangup {
    pub id: u64,
    pub kind: i32,
    pub device_id: Option<u32>,
}
