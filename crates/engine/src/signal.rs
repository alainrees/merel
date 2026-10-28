//! Signal accounts: Merel linked to a Signal account as a device, as Signal Desktop is, through
//! the separate program `merel-signal` (see [`merel_signal_link`]). Its chats show as messages
//! in one folder, so the list, the chats and search treat them like mail and name Signal as
//! where they came from.
//!
//! Linking makes the account: a fresh random key for the program's encrypted store goes in the
//! keychain as the account's password, the program shows a link as a QR code, and once the
//! Signal app on the phone scanned it, the account is there.

use std::path::PathBuf;

use merel_model::{Account, Address, Auth, Draft, FolderKind, Protocol, Server, Tls};
use merel_secrets::Secret;
pub use merel_signal_link::is_signal_address;
use merel_signal_link::{LinkStep, Linking};

use crate::{AccountStatus, Engine};

/// How often the chats of a Signal account are listed again; the program has them in memory,
/// so this costs nothing.
pub const SIGNAL_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// How the phone lists Merel among its linked devices.
const DEVICE_NAME: &str = "Merel";

/// Whether this Merel has the Signal program with it; without it, Signal is not offered.
pub fn available() -> bool {
    merel_signal_link::helper().is_some()
}

/// Whether `account` is a Signal account.
pub fn is_signal(account: &Account) -> bool {
    account.protocol == Protocol::Signal
}

/// Linking under way.
pub struct SignalLinking {
    linking: Linking,
    id: String,
    key: Secret,
    dir: PathBuf,
}

/// How far linking has come, for the window that shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// The program is getting a link from Signal.
    Starting,
    /// Show this link as a QR code for the Signal app to scan.
    Scan(String),
    /// Linked: the account is added, under this id.
    Done(String),
    Failed(String),
}

impl Engine {
    /// Starts linking a new Signal account; [`Engine::signal_link_state`] says how it goes.
    pub fn start_signal_link(&mut self) -> Result<(), String> {
        self.cancel_signal_link();
        let helper = merel_signal_link::helper().ok_or_else(|| merel_i18n::tr("signal-no-program"))?;
        let base = self.config_path.as_deref().map(crate::config::signal_dir).ok_or_else(|| merel_i18n::tr("signal-no-folder"))?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let id = format!("signal-{now}");
        let mut random = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
        let key = Secret::new(random.iter().map(|b| format!("{b:02x}")).collect::<String>());
        let dir = base.join(&id);
        let linking = Linking::start(&helper, &dir, &key, DEVICE_NAME)?;
        self.signal_link = Some(SignalLinking { linking, id, key, dir });
        self.signal_state = Some(LinkState::Starting);
        Ok(())
    }

    /// How linking goes; `None` when nothing is being linked. Adds the account once linked.
    pub fn signal_link_state(&mut self) -> Option<LinkState> {
        let step = self.signal_link.as_mut().and_then(|l| l.linking.poll());
        match step {
            Some(LinkStep::Scan(url)) => self.signal_state = Some(LinkState::Scan(url)),
            Some(LinkStep::Failed(e)) => {
                self.cancel_signal_link();
                self.signal_state = Some(LinkState::Failed(e));
            }
            Some(LinkStep::Linked { number, id }) => {
                if let Some(link) = self.signal_link.take() {
                    let account = signal_account(&link.id, number.as_deref(), &id);
                    self.signal_state = Some(match account.and_then(|a| self.add_account(a, link.key.clone())) {
                        Ok(()) => LinkState::Done(link.id),
                        Err(e) => LinkState::Failed(e),
                    });
                }
            }
            None => {}
        }
        self.signal_state.clone()
    }

    /// Stops linking, and forgets what it stored.
    pub fn cancel_signal_link(&mut self) {
        if let Some(link) = self.signal_link.take() {
            drop(link.linking);
            let _ = std::fs::remove_dir_all(&link.dir);
        }
        self.signal_state = None;
    }

    /// Lists the chats of each Signal account that is online again: new messages are already
    /// in, so they show within seconds.
    pub(crate) fn refresh_signal(&mut self) {
        let inboxes: Vec<String> = self
            .session
            .accounts
            .iter()
            .filter(|a| is_signal(a) && self.session.status.get(&a.id) == Some(&AccountStatus::Online))
            .filter_map(|a| self.session.folder_of_kind(&a.id, FolderKind::Inbox))
            .map(|f| f.id.clone())
            .collect();
        for id in inboxes {
            self.session.request_headers(&id);
        }
    }

    /// Sends a chat message over Signal: Signal encrypts it end to end itself, so it goes without
    /// OpenPGP, and as text only.
    pub(crate) fn send_signal(&mut self, account_id: &str, mut draft: Draft) -> Result<u64, String> {
        if draft.body.trim().is_empty() {
            return Err(merel_i18n::tr("chat-type-first"));
        }
        if !draft.attachments.is_empty() {
            return Err(merel_i18n::tr("signal-text-only"));
        }
        draft.encrypt = false;
        draft.sign = Some(false);
        Ok(self.session.send(account_id, draft))
    }

    /// Whether `account_id` is a Signal account.
    pub fn is_signal_account(&self, account_id: &str) -> bool {
        self.session.accounts.iter().any(|a| a.id == account_id && is_signal(a))
    }
}

/// The account for a linked Signal account with phone number `number` and service id `id`.
fn signal_account(account_id: &str, number: Option<&str>, id: &str) -> Result<Account, String> {
    let number = number.map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Signal");
    let address = Address::new(Some(number), &merel_signal_link::address_of(id)).map_err(|_| merel_i18n::tr("signal-stopped"))?;
    Ok(Account {
        id: account_id.to_string(),
        name: "Signal".into(),
        username: number.to_string(),
        address,
        protocol: Protocol::Signal,
        auth: Auth::Password,
        // Not dialled: the program connects to Signal itself.
        incoming: Some(Server { host: "signal.org".into(), port: 443, tls: Tls::Implicit }),
        outgoing: None,
        dav: None,
        signature: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linked_account_is_named_after_signal_and_its_number() {
        let a = signal_account("signal-1", Some("+31612345678"), "1b2c-uuid").unwrap();
        assert_eq!(a.name, "Signal");
        assert_eq!(a.address.email(), "1b2c-uuid@signal.invalid");
        assert_eq!(a.address.display(), "+31612345678");
        assert!(is_signal(&a));
        assert!(a.outgoing.is_none());
    }

    #[test]
    fn a_message_over_signal_goes_without_openpgp() {
        let mut e = Engine::sample();
        let a = signal_account("signal-1", None, "me").unwrap();
        e.session.accounts.push(a);
        let draft = Draft { body: "hoi".into(), encrypt: true, ..Draft::default() };
        e.send_signal("signal-1", draft).unwrap();
        let jobs = e.session.take_jobs();
        let Some(crate::Job::Send { draft, .. }) = jobs.iter().find(|j| matches!(j, crate::Job::Send { .. })) else { panic!("{jobs:?}") };
        assert!(!draft.encrypt);
        assert_eq!(draft.sign, Some(false));
        assert!(e.send_signal("signal-1", Draft::default()).is_err());
    }
}
