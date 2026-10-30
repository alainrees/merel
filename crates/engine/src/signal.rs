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
pub use merel_signal_link::{CallPhase as SignalCallPhase, CallUpdate as SignalCallUpdate, is_signal_address};
use merel_signal_link::{LinkStep, Linking};

use crate::{AccountStatus, Engine};

/// How often the chats of a Signal account are listed again; the program has them in memory,
/// so this costs nothing.
pub const SIGNAL_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// Most messages whose text is fetched at each listing.
const SIGNAL_BODIES: usize = 50;
/// Most bytes of files one Signal message may carry, as Signal allows.
const MAX_SIGNAL_FILES: usize = 100 * 1024 * 1024;
/// How long after sending a Signal message it may be changed or deleted for everyone, in
/// seconds: a day, as Signal allows.
const CHANGE_FOR: i64 = 24 * 60 * 60;
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
        // Their text is at hand in the program, so it is fetched right away: a new message in
        // an open chat shows its words, not "being fetched".
        let mut unread: Vec<(i64, String)> = self
            .session
            .messages
            .iter()
            .filter(|m| inboxes.contains(&m.folder_id) && !self.session.bodies.contains_key(&m.id))
            .map(|m| (m.date, m.id.clone()))
            .collect();
        unread.sort_by_key(|m| std::cmp::Reverse(m.0));
        for (_, id) in unread.into_iter().take(SIGNAL_BODIES) {
            self.session.request_body(&id);
        }
        for id in inboxes {
            self.session.request_headers(&id);
        }
    }

    /// Sends a chat message over Signal: Signal encrypts it end to end itself, so it goes without
    /// OpenPGP; files go along.
    pub(crate) fn send_signal(&mut self, account_id: &str, mut draft: Draft) -> Result<u64, String> {
        if draft.body.trim().is_empty() && draft.attachments.is_empty() {
            return Err(merel_i18n::tr("chat-type-first"));
        }
        if draft.attachment_bytes() > MAX_SIGNAL_FILES {
            return Err(merel_i18n::tr("signal-too-big"));
        }
        draft.encrypt = false;
        draft.sign = Some(false);
        Ok(self.session.send(account_id, draft))
    }

    /// Reacts with `emoji` to the Signal message `message_id`, as Signal does: one reaction per
    /// person, so it replaces the account's earlier one, and the same emoji again takes it back.
    /// It shows at once; the next listing confirms it.
    pub fn react(&mut self, message_id: &str, emoji: &str) -> Result<(), String> {
        self.reaction(message_id, emoji)?;
        self.dispatch();
        Ok(())
    }

    /// Queues the reaction [`Engine::react`] sends.
    fn reaction(&mut self, message_id: &str, emoji: &str) -> Result<(), String> {
        let emoji = emoji.trim();
        if emoji.is_empty() || emoji.len() > merel_model::MAX_EMOJI {
            return Err(merel_i18n::tr("signal-cannot"));
        }
        let m = self.session.message(message_id).ok_or_else(|| merel_i18n::tr("signal-unknown"))?;
        let account_id = self.session.folder(&m.folder_id).map(|f| f.account_id.clone()).ok_or_else(|| merel_i18n::tr("signal-unknown"))?;
        if !self.is_signal_account(&account_id) {
            return Err(merel_i18n::tr("signal-cannot"));
        }
        let target = m.message_id.clone().ok_or_else(|| merel_i18n::tr("signal-unknown"))?;
        let tag = m.chat.clone().unwrap_or_default();
        let again = tag.reactions.iter().any(|r| r.mine && r.emoji == emoji);
        let draft = Draft {
            to: vec![m.from.clone()],
            body: if again { String::new() } else { emoji.to_string() },
            chat: Some(merel_model::ChatTag {
                group_id: tag.group_id.clone(),
                group_name: tag.group_name.clone(),
                reaction_to: Some(target),
                ..Default::default()
            }),
            encrypt: false,
            sign: Some(false),
            ..Draft::default()
        };
        let me = self.session.accounts.iter().find(|a| a.id == account_id).map(|a| a.address.display().to_string()).unwrap_or_default();
        if let Some(chat) = self.session.messages.iter_mut().find(|x| x.id == message_id).and_then(|x| x.chat.as_mut()) {
            chat.reactions.retain(|r| !r.mine);
            if !again {
                chat.reactions.push(merel_model::Reacted { emoji: emoji.to_string(), name: me, mine: true });
            }
        }
        self.session.send(&account_id, draft);
        Ok(())
    }

    /// Whether the Signal message `message_id` is one's own and may still be changed or deleted
    /// for everyone: Signal allows it for a day.
    pub fn signal_can_change(&self, message_id: &str) -> bool {
        let Some(m) = self.session.message(message_id) else { return false };
        let signal = self.session.folder(&m.folder_id).is_some_and(|f| self.is_signal_account(&f.account_id));
        let tag = m.chat.as_ref();
        let age = chrono::Utc::now().timestamp() - m.date;
        signal && self.session.chat_is_mine(message_id) && !tag.is_some_and(|c| c.deleted) && age < CHANGE_FOR
    }

    /// Replaces the text of one's own Signal message `message_id`, for everyone in the chat, as
    /// Signal does. It shows at once; the next listing confirms it.
    pub fn signal_edit(&mut self, message_id: &str, text: &str) -> Result<(), String> {
        let text = text.trim_end();
        if text.trim().is_empty() {
            return Err(merel_i18n::tr("chat-type-first"));
        }
        let draft = self.change_draft(message_id, text, false)?;
        let (account_id, draft) = draft;
        if let Some(body) = self.session.bodies.get_mut(message_id) {
            body.text = text.to_string();
        }
        if let Some(chat) = self.session.messages.iter_mut().find(|x| x.id == message_id).and_then(|x| x.chat.as_mut()) {
            chat.edited = true;
        }
        self.session.send(&account_id, draft);
        self.dispatch();
        Ok(())
    }

    /// Deletes the Signal message `message_id`: for everyone (one's own, within a day), as
    /// Signal does, leaving a line in its place; or only here.
    pub fn signal_delete(&mut self, message_id: &str, everyone: bool) -> Result<(), String> {
        if !everyone {
            crate::commands::delete_message(&mut self.session, message_id).map_err(|_| merel_i18n::tr("signal-unknown"))?;
            self.dispatch();
            return Ok(());
        }
        let (account_id, draft) = self.change_draft(message_id, "", true)?;
        if let Some(body) = self.session.bodies.get_mut(message_id) {
            body.text = merel_i18n::tr("signal-deleted");
            body.attachments.clear();
        }
        if let Some(chat) = self.session.messages.iter_mut().find(|x| x.id == message_id).and_then(|x| x.chat.as_mut()) {
            chat.deleted = true;
            chat.reactions.clear();
            chat.quote = None;
        }
        self.session.send(&account_id, draft);
        self.dispatch();
        Ok(())
    }

    /// What goes to the Signal program to change or delete message `message_id`.
    fn change_draft(&self, message_id: &str, text: &str, delete: bool) -> Result<(String, Draft), String> {
        if !self.signal_can_change(message_id) {
            return Err(merel_i18n::tr("signal-cannot"));
        }
        let m = self.session.message(message_id).ok_or_else(|| merel_i18n::tr("signal-unknown"))?;
        let account_id = self.session.folder(&m.folder_id).map(|f| f.account_id.clone()).ok_or_else(|| merel_i18n::tr("signal-unknown"))?;
        let target = m.message_id.clone().ok_or_else(|| merel_i18n::tr("signal-unknown"))?;
        let tag = m.chat.clone().unwrap_or_default();
        let draft = Draft {
            to: m.to.clone(),
            body: text.to_string(),
            chat: Some(merel_model::ChatTag {
                group_id: tag.group_id.clone(),
                group_name: tag.group_name.clone(),
                edit_of: (!delete).then(|| target.clone()),
                delete_of: delete.then_some(target),
                ..Default::default()
            }),
            encrypt: false,
            sign: Some(false),
            ..Draft::default()
        };
        Ok((account_id, draft))
    }

    /// Who is typing in `chat` now, by name, when it is a Signal chat between two people.
    pub fn signal_typing(&self, chat: &crate::chat::Chat) -> Option<String> {
        if !self.is_signal_account(&chat.account_id) || chat.is_group() {
            return None;
        }
        chat.people.iter().find_map(|p| merel_signal_link::typing(p.email()))
    }

    /// Whether `account_id` is a Signal account.
    pub fn is_signal_account(&self, account_id: &str) -> bool {
        self.session.accounts.iter().any(|a| a.id == account_id && is_signal(a))
    }

    /// The address of the Signal account `account_id`, which its program is known by.
    pub(crate) fn signal_address(&self, account_id: &str) -> Result<String, String> {
        self.session
            .accounts
            .iter()
            .find(|a| a.id == account_id && is_signal(a))
            .map(|a| a.address.email().to_string())
            .ok_or_else(|| merel_i18n::tr("signal-stopped"))
    }

    /// Whether the Signal account `account_id` can call from this computer: Signal's calling
    /// library is in this build of its program, and the program runs.
    pub fn signal_can_call(&self, account_id: &str) -> bool {
        self.signal_address(account_id).is_ok_and(|me| merel_signal_link::can_call(&me))
    }

    /// The Signal contacts of `account_id` as people to chat with, by name, each with their
    /// phone number when Signal shares it; also whoever a chat of the account is with, so
    /// there is someone to pick before Signal has sent its list.
    pub fn signal_contacts(&self, account_id: &str) -> Vec<(merel_model::Address, String)> {
        let Ok(me) = self.signal_address(account_id) else { return Vec::new() };
        let mut out: Vec<(merel_model::Address, String)> = merel_signal_link::contacts(&me)
            .into_iter()
            .filter_map(|p| {
                let name = if p.name.trim().is_empty() { p.number.trim() } else { p.name.trim() };
                let address = merel_model::Address::new(Some(name).filter(|n| !n.is_empty()), &p.address()).ok()?;
                Some((address, p.number))
            })
            .collect();
        for chat in self.session.chats().into_iter().filter(|c| c.account_id == account_id && !c.is_group()) {
            for person in chat.people {
                if is_signal_address(person.email()) && !out.iter().any(|(a, _)| a.email().eq_ignore_ascii_case(person.email())) {
                    out.push((person, String::new()));
                }
            }
        }
        out.sort_by_cached_key(|(a, _)| a.display().to_lowercase());
        out
    }

    /// Calls the person behind the Signal address `to` (sound only); how it goes comes from
    /// [`Engine::take_signal_calls`].
    pub fn signal_call(&self, account_id: &str, to: &str) -> Result<(), String> {
        merel_signal_link::call(&self.signal_address(account_id)?, to)
    }

    pub fn signal_accept(&self, account_id: &str, call_id: u64) -> Result<(), String> {
        merel_signal_link::accept_call(&self.signal_address(account_id)?, call_id)
    }

    /// Ends the Signal call going on, or declines the one ringing.
    pub fn signal_hang_up(&self, account_id: &str) -> Result<(), String> {
        merel_signal_link::hang_up(&self.signal_address(account_id)?)
    }

    pub fn signal_mute(&self, account_id: &str, on: bool) -> Result<(), String> {
        merel_signal_link::mute(&self.signal_address(account_id)?, on)
    }

    /// How Signal calls went since the last time, with the account each is on.
    pub fn take_signal_calls(&self) -> Vec<(String, SignalCallUpdate)> {
        merel_signal_link::take_call_updates()
            .into_iter()
            .filter_map(|(me, update)| {
                let account = self.session.accounts.iter().find(|a| is_signal(a) && a.address.email().eq_ignore_ascii_case(&me))?;
                Some((account.id.clone(), update))
            })
            .collect()
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
    fn a_reaction_shows_at_once_and_the_same_emoji_takes_it_back() {
        let mut e = Engine::sample();
        e.session.accounts.push(signal_account("signal-1", Some("+31612345678"), "me").unwrap());
        e.session.folders.push(merel_model::Folder {
            id: "signal-1/signal".into(),
            account_id: "signal-1".into(),
            name: "Signal".into(),
            path: "signal".into(),
            kind: FolderKind::Inbox,
            unread: 0,
            parent: None,
        });
        let mut m = e.session.messages[0].clone();
        m.id = "signal-1/signal#5.jan".into();
        m.folder_id = "signal-1/signal".into();
        m.message_id = Some("<5.jan@signal.invalid>".into());
        m.chat = Some(merel_model::ChatTag::default());
        e.session.messages.push(m);
        e.reaction("signal-1/signal#5.jan", "👍").unwrap();
        let shown = |e: &Engine| e.session.message("signal-1/signal#5.jan").and_then(|m| m.chat.clone()).unwrap_or_default().reactions;
        assert_eq!(shown(&e).iter().map(|r| (r.emoji.as_str(), r.mine)).collect::<Vec<_>>(), [("👍", true)]);
        let jobs = e.session.take_jobs();
        let Some(crate::Job::Send { draft, .. }) = jobs.iter().find(|j| matches!(j, crate::Job::Send { .. })) else { panic!("{jobs:?}") };
        assert_eq!(draft.body, "👍");
        assert_eq!(draft.chat.as_ref().and_then(|c| c.reaction_to.as_deref()), Some("<5.jan@signal.invalid>"));
        e.reaction("signal-1/signal#5.jan", "👍").unwrap();
        assert!(shown(&e).is_empty());
        let jobs = e.session.take_jobs();
        assert!(jobs.iter().any(|j| matches!(j, crate::Job::Send { draft, .. } if draft.body.is_empty())));
        assert!(e.reaction("acc/INBOX#1", "👍").is_err());
    }

    #[test]
    fn an_answer_quotes_who_wrote_what() {
        let mut e = Engine::sample();
        let m = e.session.messages[0].clone();
        e.session.bodies.insert(m.id.clone(), merel_mime::Body { text: "Kom je ook?".into(), ..Default::default() });
        let q = e.session.chat_quote(&m.id).unwrap();
        assert_eq!(q.text, "Kom je ook?");
        assert_eq!(q.name, m.from.display());
        assert_eq!(Some(q.message_id), m.message_id);
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
