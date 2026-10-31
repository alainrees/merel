//! The chats from before linking, as Signal Desktop takes them over: when the phone offers to
//! transfer the message history while linking, it puts an encrypted archive on Signal's CDN
//! and says where. The one-time key for it comes with the link; it is kept in the encrypted
//! store only until the archive is in, as the archive's messages are kept there after.
//!
//! The archive is a message backup (libsignal's format): read here frame by frame, it gives the
//! people and groups, the chats and their messages. Only text is taken over; for files the
//! message says which file it was.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use libsignal_account_keys::BackupKey;
use libsignal_message_backup::frame::{CursorFactory, FramesReader};
use libsignal_message_backup::key::MessageBackupKey;
use presage::libsignal_service::configuration::{Endpoint, ServiceConfiguration, SignalServers};
use presage::libsignal_service::protocol::Aci;
use serde::Deserialize;
use serde_json::{Value, json};

/// The capabilities a linking device names in its link, as Signal Desktop does: with `backup5`
/// the phone offers to transfer the message history.
pub const LINK_CAPABILITIES: &str = "nopni,nopni2,backup5";

/// How long one ask for the archive may wait at the server, and how long in all.
const POLL: Duration = Duration::from_secs(300);
const WAIT: Duration = Duration::from_secs(60 * 60);

async fn db(dir: &str, key: &str) -> Result<sqlx::SqliteConnection> {
    use sqlx::ConnectOptions as _;
    let options: sqlx::sqlite::SqliteConnectOptions = format!("sqlite://{dir}/signal.db?mode=rwc").parse()?;
    let mut db = options.pragma("key", format!("'{}'", key.replace('\'', "''"))).connect().await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS merel_transfer (key BLOB NOT NULL)").execute(&mut db).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS merel_history (line TEXT NOT NULL)").execute(&mut db).await?;
    Ok(db)
}

/// Keeps the one-time key the phone gave for its message history, until the archive is in.
pub async fn keep_key(dir: &str, key: &str, backup_key: &[u8; 32]) -> Result<()> {
    let mut db = db(dir, key).await?;
    sqlx::query("DELETE FROM merel_transfer").execute(&mut db).await?;
    sqlx::query("INSERT INTO merel_transfer (key) VALUES (?)").bind(backup_key.to_vec()).execute(&mut db).await?;
    Ok(())
}

/// The messages taken over from before linking, as the lines that passed them on.
pub async fn stored(dir: &str, key: &str) -> Result<Vec<String>> {
    use sqlx::Row as _;
    let mut db = db(dir, key).await?;
    let rows = sqlx::query("SELECT line FROM merel_history ORDER BY rowid").fetch_all(&mut db).await?;
    rows.iter().map(|r| r.try_get::<String, _>(0).map_err(Into::into)).collect()
}

/// The groups of the chats taken over, by Merel's id for them, for sending to them.
pub async fn groups(dir: &str, key: &str) -> Result<Vec<[u8; 32]>> {
    let mut found = Vec::new();
    for line in stored(dir, key).await? {
        let value: Option<Value> = serde_json::from_str(&line).ok();
        let k = value.as_ref().and_then(|v| v.get("master_key")).and_then(Value::as_str).and_then(|k| hex::decode(k).ok());
        if let Some(k) = k.and_then(|k| <[u8; 32]>::try_from(k).ok())
            && !found.contains(&k)
        {
            found.push(k);
        }
    }
    Ok(found)
}

/// The account, as it signs in to Signal: from presage's registration data.
pub struct Login {
    pub servers: SignalServers,
    pub aci: uuid::Uuid,
    pub username: String,
    pub password: String,
}

impl Login {
    pub fn from_registration(registration: &Value) -> Result<Login> {
        #[derive(Deserialize)]
        struct R {
            signal_servers: SignalServers,
            uuid: uuid::Uuid,
            password: String,
            device_id: Option<u32>,
        }
        let r: R = serde_json::from_value(registration.clone())?;
        let username = match r.device_id {
            Some(d) if d != 1 => format!("{}.{d}", r.uuid),
            _ => r.uuid.to_string(),
        };
        Ok(Login { servers: r.signal_servers, aci: r.uuid, username, password: r.password })
    }
}

/// When a key is kept: waits for the phone's archive, takes in its messages and keeps them, and
/// forgets the key. Returns the messages to pass on now, as event lines; none without a key.
pub async fn take_over(dir: String, key: String, login: Login) -> Result<Vec<String>> {
    let (dir, key) = (dir.as_str(), key.as_str());
    use sqlx::Row as _;
    let mut db = db(dir, key).await?;
    let Some(row) = sqlx::query("SELECT key FROM merel_transfer").fetch_optional(&mut db).await? else { return Ok(Vec::new()) };
    let backup_key: Vec<u8> = row.try_get(0)?;
    let backup_key = <[u8; 32]>::try_from(backup_key).map_err(|_| anyhow!("the history key is not 32 bytes"))?;
    eprintln!("merel-signal: waiting for the message history from the phone");
    let lines = match fetch(&login).await? {
        Some(archive) => {
            eprintln!("merel-signal: message history in, {} kB", archive.len() / 1024);
            let frames = read(&archive, &backup_key, login.aci).await?;
            let lines = messages(&frames);
            for line in &lines {
                sqlx::query("INSERT INTO merel_history (line) VALUES (?)").bind(line).execute(&mut db).await?;
            }
            eprintln!("merel-signal: {} messages from before linking taken over", lines.len());
            lines
        }
        None => {
            eprintln!("merel-signal: the phone sends no message history");
            Vec::new()
        }
    };
    sqlx::query("DELETE FROM merel_transfer").execute(&mut db).await?;
    Ok(lines)
}

fn client(cfg: &ServiceConfiguration) -> Result<reqwest::Client> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Ok(reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(cfg.certificate_authority.as_bytes())?])
        .connect_timeout(Duration::from_secs(30))
        .build()?)
}

/// Asks the server for the archive until the phone put it there, then fetches it from the
/// CDN. None when the phone sends none.
async fn fetch(login: &Login) -> Result<Option<Vec<u8>>> {
    let cfg = ServiceConfiguration::from(login.servers);
    let http = client(&cfg)?;
    let url = Endpoint::service(format!("/v1/devices/transfer_archive?timeout={}", POLL.as_secs())).into_url(&cfg)?;
    let started = Instant::now();
    let (cdn, path) = loop {
        if started.elapsed() > WAIT {
            bail!("the phone did not send the message history within an hour");
        }
        let asked = Instant::now();
        let response = http.get(url.clone()).basic_auth(&login.username, Some(&login.password)).timeout(POLL + Duration::from_secs(30)).send().await;
        match response {
            Ok(r) if r.status() == reqwest::StatusCode::NO_CONTENT => {}
            Ok(r) if r.status().is_success() => {
                let answer: Value = r.json().await?;
                if let Some(error) = answer.get("error").and_then(Value::as_str) {
                    eprintln!("merel-signal: the phone answered {error}");
                    return Ok(None);
                }
                let cdn = answer.get("cdn").and_then(Value::as_u64).context("no cdn")?;
                let path = answer.get("key").and_then(Value::as_str).context("no key")?.to_string();
                break (u32::try_from(cdn)?, path);
            }
            Ok(r) => eprintln!("merel-signal: asking for the message history: {}", r.status()),
            Err(e) => eprintln!("merel-signal: asking for the message history: {e}"),
        }
        // Never more often than every few seconds.
        if let Some(rest) = Duration::from_secs(5).checked_sub(asked.elapsed()) {
            tokio::time::sleep(rest).await;
        }
    };
    let path = format!("/attachments/{}", urlencode(&path));
    let url = Endpoint::cdn(cdn, path).into_url(&cfg)?;
    let mut tries = 0;
    loop {
        tries += 1;
        match http.get(url.clone()).timeout(Duration::from_secs(30 * 60)).send().await.and_then(reqwest::Response::error_for_status) {
            Ok(r) => return Ok(Some(r.bytes().await?.to_vec())),
            Err(e) if tries < 5 => {
                eprintln!("merel-signal: fetching the message history: {e}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Decrypts the archive with the one-time key and gives its frames as JSON.
async fn read(archive: &[u8], backup_key: &[u8; 32], aci: uuid::Uuid) -> Result<Vec<Value>> {
    let backup_key: BackupKey = BackupKey(*backup_key);
    let backup_id = backup_key.derive_backup_id(&Aci::from(aci));
    let key = MessageBackupKey::derive(&backup_key, &backup_id, None);
    let reader = FramesReader::new(&key, CursorFactory::new(archive)).await.map_err(|e| anyhow!("the message history could not be opened: {e}"))?;
    let frames =
        libsignal_message_backup::backup::convert_to_json(reader).await.map_err(|e| anyhow!("the message history could not be read: {e}"))?;
    Ok(frames.iter().filter_map(|f| serde_json::from_str(f).ok()).collect())
}

/// Who a recipient in the archive is.
enum Who {
    Me,
    Person { id: String, name: String },
    Group { master_key: [u8; 32], title: String },
}

fn number(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn bytes(v: &Value) -> Option<Vec<u8>> {
    v.as_str().and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
}

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

/// The archive's messages as the event lines that pass a message on, oldest first, with the
/// names the phone knows people and groups by.
fn messages(frames: &[Value]) -> Vec<String> {
    let mut who: HashMap<u64, Who> = HashMap::new();
    let mut chats: HashMap<u64, u64> = HashMap::new();
    let mut lines = Vec::new();
    for frame in frames {
        if let Some(r) = frame.get("recipient") {
            let Some(id) = r.get("id").and_then(number) else { continue };
            if r.get("self").is_some() {
                who.insert(id, Who::Me);
            } else if let Some(c) = r.get("contact") {
                let Some(aci) = c.get("aci").and_then(bytes).and_then(|b| uuid::Uuid::from_slice(&b).ok()) else { continue };
                let nick = c.get("nickname").map(|n| format!("{} {}", text(n.get("given")), text(n.get("family")))).unwrap_or_default();
                let system = format!("{} {}", text(c.get("systemGivenName")), text(c.get("systemFamilyName")));
                let profile = format!("{} {}", text(c.get("profileGivenName")), text(c.get("profileFamilyName")));
                let phone = c.get("e164").and_then(number).map(|n| format!("+{n}")).unwrap_or_default();
                let name = [nick, system, profile, phone].into_iter().map(|n| n.trim().to_string()).find(|n| !n.is_empty()).unwrap_or_default();
                let id_str = aci.to_string();
                who.insert(id, Who::Person { name, id: id_str });
            } else if let Some(g) = r.get("group") {
                let Some(master_key) = g.get("masterKey").and_then(bytes).and_then(|k| <[u8; 32]>::try_from(k).ok()) else { continue };
                let title = text(g.get("snapshot").and_then(|s| s.get("title")).and_then(|t| t.get("title")));
                who.insert(id, Who::Group { master_key, title });
            }
        } else if let Some(c) = frame.get("chat") {
            if let (Some(id), Some(r)) = (c.get("id").and_then(number), c.get("recipientId").and_then(number)) {
                chats.insert(id, r);
            }
        } else if let Some(item) = frame.get("chatItem") {
            let Some(line) = message(item, &who, &chats) else { continue };
            lines.push(line);
        }
    }
    lines
}

fn message(item: &Value, who: &HashMap<u64, Who>, chats: &HashMap<u64, u64>) -> Option<String> {
    let standard = item.get("standardMessage")?;
    let chat = who.get(chats.get(&item.get("chatId").and_then(number)?)?)?;
    let author = who.get(&item.get("authorId").and_then(number)?)?;
    let timestamp = item.get("dateSent").and_then(number)?;
    let mut body = text(standard.get("text").and_then(|t| t.get("body")));
    for a in standard.get("attachments").and_then(Value::as_array).into_iter().flatten() {
        let name = text(a.get("pointer").and_then(|p| p.get("fileName")));
        let line = if name.is_empty() { "📎".to_string() } else { format!("📎 {name}") };
        body = if body.is_empty() { line } else { format!("{body}\n{line}") };
    }
    if body.is_empty() {
        return None;
    }
    let (thread, thread_name, group, master_key) = match chat {
        Who::Person { id, name } => (id.clone(), name.clone(), false, None),
        Who::Group { master_key, title } => (crate::group_id(master_key), title.clone(), true, Some(hex::encode(master_key))),
        Who::Me => return None,
    };
    let (sender, sender_name, mine) = match author {
        Who::Me => (String::new(), String::new(), true),
        Who::Person { id, name } => (id.clone(), name.clone(), false),
        Who::Group { .. } => return None,
    };
    let mut line = json!({
        "type": "message",
        "thread": thread,
        "thread_name": thread_name,
        "group": group,
        "sender": sender,
        "sender_name": sender_name,
        "mine": mine,
        "timestamp": timestamp,
        "text": body,
        "attachments": [],
    });
    if let Some(k) = master_key {
        line["master_key"] = k.into();
    }
    Some(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_gives_its_chats_messages_with_names() {
        let aci = "659aa5f4-a28d-fcc1-1ea1-b997537a3d95";
        let aci_b64 = base64::engine::general_purpose::STANDARD.encode(uuid::Uuid::parse_str(aci).unwrap().as_bytes());
        let key_b64 = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let frames: Vec<Value> = [
            json!({"recipient": {"id": "1", "self": {}}}),
            json!({"recipient": {"id": "2", "contact": {"aci": aci_b64, "e164": "31612345678", "systemGivenName": "Sofie", "systemFamilyName": "Stickel"}}}),
            json!({"recipient": {"id": "3", "group": {"masterKey": key_b64, "snapshot": {"title": {"title": "CISO Vrienden"}}}}}),
            json!({"chat": {"id": "10", "recipientId": "2"}}),
            json!({"chat": {"id": "11", "recipientId": "3"}}),
            json!({"chatItem": {"chatId": "10", "authorId": "2", "dateSent": "1729000000000", "incoming": {}, "standardMessage": {"text": {"body": "Jij ook alvast een goed weekend"}}}}),
            json!({"chatItem": {"chatId": "11", "authorId": "1", "dateSent": "1729000001000", "outgoing": {}, "standardMessage": {"text": {"body": "Hoi"}, "attachments": [{"pointer": {"fileName": "foto.jpg"}}]}}}),
            json!({"chatItem": {"chatId": "11", "authorId": "1", "dateSent": "1729000002000", "outgoing": {}, "updateMessage": {}}}),
        ]
        .into();
        let lines = messages(&frames);
        assert_eq!(lines.len(), 2, "the group update is no message");
        let first: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(first["thread"], aci);
        assert_eq!(first["thread_name"], "Sofie Stickel");
        assert_eq!(first["mine"], false);
        assert_eq!(first["timestamp"], 1_729_000_000_000u64);
        let second: Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(second["group"], true);
        assert_eq!(second["thread"], crate::group_id(&[7; 32]));
        assert_eq!(second["thread_name"], "CISO Vrienden");
        assert_eq!(second["mine"], true);
        assert_eq!(second["text"], "Hoi\n📎 foto.jpg");
        assert_eq!(second["master_key"], hex::encode([7u8; 32]));
    }

    #[test]
    fn the_login_of_a_linked_device_carries_its_device_number() {
        let r = json!({"signal_servers": "Production", "uuid": "659aa5f4-a28d-fcc1-1ea1-b997537a3d95", "password": "pw", "device_id": 3, "phone_number": {}});
        let login = Login::from_registration(&r).unwrap();
        assert_eq!(login.username, "659aa5f4-a28d-fcc1-1ea1-b997537a3d95.3");
        assert_eq!(login.password, "pw");
    }
}
