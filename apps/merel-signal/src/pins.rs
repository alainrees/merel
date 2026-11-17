//! The chats pinned on the phone: Signal keeps them in the account record of its storage
//! service, encrypted with a key only the account's devices have. A linked device reads them
//! from there, as Signal Desktop does; it reads, it never writes, so nothing Merel does can
//! change the account's settings on the phone.

use anyhow::{Result, anyhow};
use presage::libsignal_service::StorageService;
use presage::libsignal_service::configuration::{ServiceCredentials, SignalServers};
use presage::libsignal_service::master_key::StorageServiceKey;
use presage::libsignal_service::protocol::ServiceId;
use presage::libsignal_service::push_service::PushService;
use presage::libsignal_service::utils::TryIntoE164 as _;
use presage::proto::account_record::pinned_conversation::Identifier;
use presage::proto::{AccountRecord, manifest_record, storage_record};
use presage::store::Store;
use serde::Deserialize;

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

/// What it takes to read the storage service: how to sign in, and the key to its contents.
pub struct Reader {
    servers: SignalServers,
    credentials: ServiceCredentials,
    key: StorageServiceKey,
}

impl Reader {
    /// From the registration (as `lookup_registration` gives it) and the store's master key;
    /// `None` while the phone has not sent the master key yet.
    pub async fn new<S: Store>(store: &S, registration: serde_json::Value) -> Result<Option<Reader>> {
        let Some(master) = store.fetch_master_key().await.map_err(|e| anyhow!("{e}"))? else { return Ok(None) };
        let r: Registration = serde_json::from_value(registration)?;
        let credentials = ServiceCredentials {
            aci: Some(r.aci),
            pni: Some(r.pni),
            phonenumber: r.phone_number.as_str().try_into_e164().map_err(|_| anyhow!("the account's number is not a phone number"))?,
            password: Some(r.password),
            device_id: r.device_id.and_then(|d| u8::try_from(d).ok()).and_then(|d| d.try_into().ok()),
        };
        Ok(Some(Reader { servers: r.signal_servers, credentials, key: StorageServiceKey::from_master_key(&master) }))
    }

    /// The pinned chats, in the phone's order, as thread ids; `None` when the manifest is still
    /// at `version`, so nothing changed.
    pub async fn pinned(self, version: Option<u64>) -> Result<Option<(u64, Vec<String>)>> {
        let push = PushService::new(self.servers, Some(self.credentials), "merel-signal");
        let storage = StorageService::new(push, self.key).await?;
        let manifest = match version {
            Some(v) => match storage.manifest_if_changed(v).await? {
                Some(m) => m,
                None => return Ok(None),
            },
            None => storage.manifest().await?,
        };
        let keys: Vec<Vec<u8>> =
            manifest.identifiers.iter().filter(|id| id.r#type() == manifest_record::identifier::Type::Account).map(|id| id.raw.clone()).collect();
        if keys.is_empty() {
            return Ok(Some((manifest.version, Vec::new())));
        }
        let records = storage.read_items(keys, Some(&manifest.record_ikm)).await?;
        let account = records.into_iter().find_map(|r| match r.record {
            Some(storage_record::Record::Account(a)) => Some(a),
            _ => None,
        });
        Ok(Some((manifest.version, account.map(|a| threads(&a)).unwrap_or_default())))
    }
}

/// The thread ids of the chats `account` has pinned, in order. Old-style groups and Signal's
/// release notes have no chat in Merel and are left out.
pub fn threads(account: &AccountRecord) -> Vec<String> {
    let mut out = Vec::new();
    for pinned in &account.pinned_conversations {
        let thread = match &pinned.identifier {
            Some(Identifier::Contact(c)) => ServiceId::parse_from_service_id_binary(&c.service_id_binary)
                .or_else(|| ServiceId::parse_from_service_id_string(&c.service_id))
                .map(|id| id.service_id_string()),
            Some(Identifier::GroupMasterKey(key)) => <[u8; 32]>::try_from(key.as_slice()).ok().map(|k| crate::group_id(&k)),
            _ => None,
        };
        if let Some(t) = thread
            && !out.contains(&t)
        {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use presage::proto::account_record::PinnedConversation;
    use presage::proto::account_record::pinned_conversation::Contact;

    #[test]
    fn pinned_chats_come_in_the_phones_order_people_and_groups() {
        let person = "9d0652a3-dcc3-4d11-975f-74d61598733f";
        let key = [7u8; 32];
        let account = AccountRecord {
            pinned_conversations: vec![
                PinnedConversation { identifier: Some(Identifier::GroupMasterKey(key.to_vec())) },
                PinnedConversation { identifier: Some(Identifier::Contact(Contact { service_id: person.into(), ..Default::default() })) },
                PinnedConversation { identifier: Some(Identifier::LegacyGroupId(vec![1, 2, 3])) },
                PinnedConversation { identifier: None },
            ],
            ..Default::default()
        };
        assert_eq!(threads(&account), vec![crate::group_id(&key), person.to_string()]);
    }

    #[test]
    fn a_person_given_only_in_binary_is_found_too() {
        let id = uuid::Uuid::parse_str("9d0652a3-dcc3-4d11-975f-74d61598733f").unwrap();
        let account = AccountRecord {
            pinned_conversations: vec![PinnedConversation {
                identifier: Some(Identifier::Contact(Contact { service_id_binary: id.as_bytes().to_vec(), ..Default::default() })),
            }],
            ..Default::default()
        };
        assert_eq!(threads(&account), vec![id.to_string()]);
    }
}
