//! Finds out which phone numbers of the address book are on Signal, as Signal itself does:
//! through Signal's contact discovery service, where the numbers are looked up inside an
//! enclave that not even Signal can look into.
//!
//! A program of its own, next to merel-signal: the discovery service brings BoringSSL, which
//! cannot be linked next to the OpenSSL of the encrypted store (as with calls, see wire.rs).
//! merel-signal starts it and writes one JSON line to its input: the account's registration as
//! the store keeps it, and the numbers (as +31612345678). It answers with one JSON line: the
//! numbers on Signal with their service id, or what went wrong.

use anyhow::{Context as _, Result, anyhow};
use presage::libsignal_service::configuration::{ServiceCredentials, SignalServers};
use presage::libsignal_service::push_service::PushService;
use presage::libsignal_service::utils::TryIntoE164 as _;
use presage::libsignal_service::websocket::{self, SignalWebSocket, directory::LookupRequest};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Deserialize)]
struct Asked {
    registration: Registration,
    numbers: Vec<String>,
}

/// The parts of presage's registration data that sign in to Signal.
#[derive(Deserialize)]
struct Registration {
    signal_servers: SignalServers,
    phone_number: String,
    #[serde(rename = "uuid")]
    aci: uuid::Uuid,
    #[serde(default)]
    pni: uuid::Uuid,
    password: String,
    device_id: Option<u32>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Answer {
    Found { people: Vec<Found> },
    Error { message: String },
}

#[derive(Serialize)]
struct Found {
    id: String,
    number: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let answer = match lookup().await {
        Ok(people) => Answer::Found { people },
        Err(e) => Answer::Error { message: format!("{e:#}") },
    };
    if let Ok(line) = serde_json::to_string(&answer) {
        println!("{line}");
    }
}

async fn lookup() -> Result<Vec<Found>> {
    let line = BufReader::new(tokio::io::stdin()).lines().next_line().await?.context("nothing asked")?;
    let asked: Asked = serde_json::from_str(&line)?;
    let r = asked.registration;
    let credentials = ServiceCredentials {
        aci: Some(r.aci),
        pni: Some(r.pni),
        phonenumber: r.phone_number.as_str().try_into_e164().map_err(|_| anyhow!("the account's number is not a phone number"))?,
        password: Some(r.password),
        device_id: r.device_id.and_then(|d| u8::try_from(d).ok()).and_then(|d| d.try_into().ok()),
    };
    let mut push = PushService::new(r.signal_servers, Some(credentials.clone()), "merel-signal-lookup");
    let mut ws: SignalWebSocket<websocket::Identified> = push.ws("/v1/websocket/", "/v1/keepalive", &[], Some(credentials)).await?;
    let request = LookupRequest { new_e164s: asked.numbers.iter().filter_map(|n| n.as_str().try_into_e164().ok()).collect(), ..Default::default() };
    let found = ws.discover_contacts(request).await?;
    Ok(found.into_iter().filter_map(|(number, id)| Some(Found { id: id?.service_id_string(), number: number.to_string() })).collect())
}
