//! Invitations, the way Delta Chat makes them (Secure-Join, setup-contact): a link, or the QR
//! code of it, holds the inviter's key fingerprint, address and name and two random tokens.
//! Whoever opens it writes to the inviter, and after four short messages both sides have each
//! other's key, checked against the fingerprint in the link rather than taken on trust. From
//! then on they chat end-to-end encrypted, also with someone on Delta Chat.
//!
//! 1. The joiner sends `vc-request` with the first token, unencrypted, with its key.
//! 2. The inviter answers `vc-auth-required`, encrypted to that key.
//! 3. The joiner checks that the answer is signed with the key in the link and sends
//!    `vc-request-with-auth` with the second token and its own fingerprint, encrypted.
//! 4. The inviter checks the token and the fingerprint, keeps the joiner's key as verified and
//!    confirms with `vc-contact-confirm`.
//!
//! Merel's own links leave out `v=3`, so Delta Chat takes these steps rather than its newer
//! ones with a shared password. Every encrypted step shows the other side its own key
//! (`Autocrypt-Gossip`), which Delta Chat checks, and Merel checks that too.

use std::collections::BTreeMap;

use merel_mime::{Body, Encryption, ParsedHeader};
use merel_model::{Address, ChatTag, Draft, FolderKind, Handshake};
use serde::{Deserialize, Serialize};

use crate::drafts::now;
use crate::openpgp::Carried;
use crate::{Engine, SAMPLE_ACCOUNT, config};

/// Where Delta Chat's invitation links point; the page tells people without the app where to
/// get it, and the part after `#` never leaves their device.
pub const INVITE_LINK: &str = "https://i.delta.chat/#";
/// The handled handshake messages remembered, so none is answered twice.
const MAX_HANDLED: usize = 500;
/// The invitations opened at a time that wait for an answer.
const MAX_JOINING: usize = 20;
/// How long an opened invitation waits for the other side: a week.
const JOIN_FOR: i64 = 7 * 24 * 3600;
/// The chat messages held back at a time.
const MAX_WAITING: usize = 100;
/// How long before Merel mails the same person an invitation again: a week.
const INVITE_AGAIN: i64 = 7 * 24 * 3600;

/// What an invitation link says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    /// The inviter's key, upper-case hex.
    pub fingerprint: String,
    pub addr: String,
    pub name: String,
    pub invitenumber: String,
    pub auth: String,
}

impl Invite {
    /// The link, as Delta Chat writes it.
    pub fn link(&self) -> String {
        format!("{INVITE_LINK}{}&i={}&s={}&a={}&n={}", self.fingerprint, self.invitenumber, self.auth, encode(&self.addr), encode(&self.name))
    }
}

/// Percent-encodes `text` as Delta Chat does in its links: letters, digits, `.` and `_` stay,
/// a space becomes `+`.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b' ' => out.push('+'),
            b if b.is_ascii_alphanumeric() || b == b'.' || b == b'_' => out.push(char::from(b)),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| char::from(b).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi * 16 + lo) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Reads an invitation link (`https://i.delta.chat/#…`) or what a QR code of one says
/// (`OPENPGP4FPR:…`).
pub fn parse_invite(text: &str) -> Result<Invite, String> {
    let text = text.trim();
    let wrong = || merel_i18n::tr("invite-not-a-link");
    let rest = if let Some(rest) = strip_prefix_ignore_case(text, INVITE_LINK) {
        rest
    } else if let Some(rest) = strip_prefix_ignore_case(text, "OPENPGP4FPR:") {
        rest
    } else {
        return Err(wrong());
    };
    let (fingerprint, params) = rest.split_once(['&', '#']).ok_or_else(wrong)?;
    let fingerprint = merel_model::fingerprint_of(fingerprint).ok_or_else(wrong)?;
    let mut fields: BTreeMap<&str, String> = BTreeMap::new();
    for pair in params.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            fields.entry(k).or_insert_with(|| decode(v));
        }
    }
    if fields.contains_key("x") {
        return Err(merel_i18n::tr("invite-group"));
    }
    let addr = fields.get("a").and_then(|a| Address::new(None, a.trim()).ok()).ok_or_else(wrong)?.email().to_lowercase();
    let token = |k: &str| fields.get(k).and_then(|v| merel_model::chat_token(v)).ok_or_else(wrong);
    let (invitenumber, auth) = (token("i")?, token("s")?);
    let name = merel_model::clean_text(fields.get("n").map(|n| n.trim()).unwrap_or_default()).chars().take(100).collect();
    Ok(Invite { fingerprint, addr, name, invitenumber, auth })
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

/// A random token of `n` characters.
fn new_token(n: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut random = vec![0u8; n];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
    random.iter().map(|b| char::from(ALPHABET[usize::from(*b) % ALPHABET.len()])).collect()
}

/// An account's own invitation tokens: the same link keeps working, until it is renewed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Tokens {
    invitenumber: String,
    auth: String,
}

/// An invitation the person opened that waits for the other side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Joining {
    account_id: String,
    invite: Invite,
    since: i64,
}

/// `invites.json`: the accounts' own tokens, the invitations under way, the verified keys and
/// the handshake messages already answered.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Invites {
    #[serde(default)]
    own: BTreeMap<String, Tokens>,
    #[serde(default)]
    joining: Vec<Joining>,
    /// Address → the fingerprint checked through an invitation.
    #[serde(default)]
    verified: BTreeMap<String, String>,
    #[serde(default)]
    handled: Vec<String>,
    /// Chat messages held back until everyone in them has a key.
    #[serde(default)]
    waiting: Vec<Waiting>,
    /// Address → when Merel last mailed it an invitation by itself.
    #[serde(default)]
    invited: BTreeMap<String, i64>,
}

/// A chat message held back because someone in it has no key yet. Merel mailed them an
/// invitation; the message goes out, encrypted, once their key comes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    pub account_id: String,
    pub draft: Draft,
    /// When it was written, in seconds since the Unix epoch.
    pub since: i64,
}

impl Waiting {
    /// The chat the message belongs to.
    pub fn chat_id(&self) -> String {
        match self.draft.chat.as_ref().and_then(|c| c.group_id.as_deref()) {
            Some(group) => crate::chat::group_chat_id(&self.account_id, group),
            None => crate::chat::person_chat_id(&self.account_id, self.draft.to.first().map(Address::email).unwrap_or("")),
        }
    }
}

/// How an invitation one opened is getting on, for the chat start page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub account_id: String,
    pub addr: String,
    pub name: String,
}

impl Engine {
    /// The invitation link of `account_id`, turning chat on for it first. Anyone who opens it
    /// in Merel or Delta Chat gets a verified chat with the account.
    pub fn invite_link(&mut self, account_id: &str) -> Result<String, String> {
        let info = self.enable_chat(account_id)?;
        let account = self.session.accounts.iter().find(|a| a.id == account_id).ok_or(merel_i18n::tr("account-gone"))?;
        let (addr, name) = (account.address.email().to_lowercase(), account.address.display().to_string());
        let name = if name.eq_ignore_ascii_case(&addr) { String::new() } else { name };
        let tokens =
            self.invites.own.entry(account_id.to_string()).or_insert_with(|| Tokens { invitenumber: new_token(24), auth: new_token(24) }).clone();
        self.save_invites();
        Ok(Invite { fingerprint: info.fingerprint, addr, name, invitenumber: tokens.invitenumber, auth: tokens.auth }.link())
    }

    /// Sends a chat message, or, when someone in it has no key yet, mails them an invitation
    /// (only the link, never the message) and holds the message until their key comes, as it
    /// does through the invitation or any encrypted mail from them. `Ok(None)` then; see
    /// [`Engine::waiting`].
    pub fn send_or_invite(&mut self, account_id: &str, draft: Draft) -> Result<Option<u64>, String> {
        let sent = self.hold_or_send(account_id, draft);
        self.dispatch();
        sent
    }

    /// [`Engine::send_or_invite`] without handing the mail to the worker yet.
    pub(crate) fn hold_or_send(&mut self, account_id: &str, draft: Draft) -> Result<Option<u64>, String> {
        if draft.chat.is_none() || !draft.encrypt {
            return Err(merel_i18n::tr("chat-encrypted-only"));
        }
        if draft.body.trim().is_empty() && draft.attachments.is_empty() {
            return Err(merel_i18n::tr("chat-type-first"));
        }
        if !self.chat_ready(account_id) {
            return Err(merel_i18n::tr("chat-turn-on-first"));
        }
        let missing = self.keys.openpgp.missing(draft.to.iter(), now());
        if missing.is_empty() {
            return Ok(Some(self.session.send(account_id, draft)));
        }
        if self.invites.waiting.len() >= MAX_WAITING {
            return Err(crate::chat::missing_chat_text(&missing));
        }
        let link = self.invite_link(account_id)?;
        let at = now();
        for addr in &missing {
            let key = addr.to_lowercase();
            if self.invites.invited.get(&key).is_some_and(|t| at - t < INVITE_AGAIN) {
                continue;
            }
            let Some(person) = draft.to.iter().find(|a| a.email().eq_ignore_ascii_case(addr)).cloned() else { continue };
            let invitation = Draft {
                to: vec![person],
                subject: merel_i18n::tr("chat-invite-mail-subject"),
                body: format!("{}\n\n{link}", merel_i18n::tr("chat-invite-mail-text")),
                chat: Some(ChatTag { invite: true, ..ChatTag::default() }),
                encrypt: false,
                ..Draft::default()
            };
            self.session.send(account_id, invitation);
            self.invites.invited.insert(key, at);
        }
        self.invites.waiting.push(Waiting { account_id: account_id.to_string(), draft, since: at });
        self.save_invites();
        Ok(None)
    }

    /// The chat messages held back for a key, oldest first.
    pub fn waiting(&self) -> &[Waiting] {
        &self.invites.waiting
    }

    /// Drops the messages held back in `chat_id`.
    pub fn cancel_waiting(&mut self, chat_id: &str) {
        self.invites.waiting.retain(|w| w.chat_id() != chat_id);
        self.save_invites();
    }

    /// Sends the held-back messages whose people all have a key by now.
    pub(crate) fn send_waiting(&mut self) {
        if self.invites.waiting.is_empty() {
            return;
        }
        let at = now();
        let (ready, rest): (Vec<Waiting>, Vec<Waiting>) =
            std::mem::take(&mut self.invites.waiting).into_iter().partition(|w| self.keys.openpgp.missing(w.draft.to.iter(), at).is_empty());
        self.invites.waiting = rest;
        if ready.is_empty() {
            return;
        }
        for w in ready {
            self.session.send(&w.account_id, w.draft);
        }
        self.save_invites();
    }

    /// Makes a new invitation link for `account_id`; the old one stops working.
    pub fn renew_invite_link(&mut self, account_id: &str) -> Result<String, String> {
        self.invites.own.remove(account_id);
        self.invite_link(account_id)
    }

    /// Takes the invitation `text` (a link or what its QR code says) from `account_id`: the
    /// handshake starts, and the chat with the inviter follows once it is done. Gives the id of
    /// that chat.
    pub fn accept_invite(&mut self, account_id: &str, text: &str) -> Result<String, String> {
        let chat = self.join(account_id, text)?;
        self.dispatch();
        Ok(chat)
    }

    /// [`Self::accept_invite`], leaving the first step in the session's queue.
    fn join(&mut self, account_id: &str, text: &str) -> Result<String, String> {
        let invite = parse_invite(text)?;
        if self.session.accounts.iter().any(|a| a.address.email().eq_ignore_ascii_case(&invite.addr)) {
            return Err(merel_i18n::tr("invite-own-account"));
        }
        let own = self.enable_chat(account_id)?;
        let now = crate::drafts::now();
        self.invites.joining.retain(|j| now - j.since < JOIN_FOR && !(j.account_id == account_id && j.invite.addr == invite.addr));
        if self.invites.joining.len() >= MAX_JOINING {
            self.invites.joining.remove(0);
        }
        let to = Address::new((!invite.name.is_empty()).then_some(invite.name.as_str()), &invite.addr).map_err(|e| e.to_string())?;
        // With the inviter's key known already, the token can go at once, encrypted.
        let known = self.pgp_key_of(&invite.addr).is_some_and(|k| k.fingerprint == invite.fingerprint);
        if known {
            self.send_step(
                account_id,
                to,
                Handshake {
                    step: "vc-request-with-auth".into(),
                    auth: Some(invite.auth.clone()),
                    fingerprint: Some(own.fingerprint),
                    ..Handshake::default()
                },
            );
        } else {
            self.send_step(
                account_id,
                to,
                Handshake { step: "vc-request".into(), invitenumber: Some(invite.invitenumber.clone()), ..Handshake::default() },
            );
        }
        let chat = crate::chat::person_chat_id(account_id, &invite.addr);
        self.invites.joining.push(Joining { account_id: account_id.to_string(), invite, since: now });
        self.save_invites();
        Ok(chat)
    }

    /// The invitations opened that wait for the other side.
    pub fn pending_invites(&self) -> Vec<Pending> {
        let now = crate::drafts::now();
        self.invites
            .joining
            .iter()
            .filter(|j| now - j.since < JOIN_FOR)
            .map(|j| Pending { account_id: j.account_id.clone(), addr: j.invite.addr.clone(), name: j.invite.name.clone() })
            .collect()
    }

    /// Whether the key Merel has for `addr` was checked through an invitation.
    pub fn is_verified(&self, addr: &str) -> bool {
        let addr = addr.trim().to_lowercase();
        self.invites.verified.get(&addr).is_some_and(|fp| self.pgp_key_of(&addr).is_some_and(|k| &k.fingerprint == fp))
    }

    /// Sends the handshake step `step` from `account_id` to `to`: the first one as it is, the
    /// others encrypted and signed.
    fn send_step(&mut self, account_id: &str, to: Address, step: Handshake) {
        let encrypt = step.step != "vc-request";
        let draft = Draft {
            to: vec![to],
            subject: crate::chat::CHAT_SUBJECT.into(),
            body: format!("Secure-Join: {}", step.step),
            sign: Some(encrypt),
            encrypt,
            chat: Some(ChatTag { handshake: Some(step), ..ChatTag::default() }),
            ..Draft::default()
        };
        // Sent with the next round of jobs.
        self.session.send(account_id, draft);
    }

    /// Answers the handshake message `message_id` that just came in, opened: `header` as it
    /// reads inside, `body` with its signature, and the keys it carried inside.
    pub(crate) fn handle_handshake(&mut self, message_id: &str, header: &ParsedHeader, body: &Body, carried: Option<&Carried>) {
        let Some(step) = header.chat.as_ref().and_then(|c| c.handshake.clone()) else { return };
        let Some(m) = self.session.message(message_id) else { return };
        let Some(folder) = self.session.folder(&m.folder_id).filter(|f| f.kind == FolderKind::Inbox) else { return };
        let account_id = folder.account_id.clone();
        let Some(account) = self.session.accounts.iter().find(|a| a.id == account_id && a.id != SAMPLE_ACCOUNT) else { return };
        let me = account.address.email().to_lowercase();
        let from = m.from.clone();
        let addr = from.email().to_lowercase();
        let Some(own) = self.pgp_key(&account_id) else { return };
        if addr == me {
            return;
        }
        let seen = m.message_id.clone().unwrap_or_else(|| message_id.to_string());
        if self.invites.handled.contains(&seen) {
            return;
        }
        self.invites.handled.push(seen);
        if self.invites.handled.len() > MAX_HANDLED {
            self.invites.handled.remove(0);
        }
        let encrypted = body.encrypted == Some(Encryption::Opened);
        // The others' steps after the first are encrypted, signed, and show this account's key.
        let signer = signed_by(body, carried);
        let shows_me = carried.is_some_and(|c| c.gossip.iter().any(|(a, k)| *a == me && k.info().fingerprint == own.fingerprint));
        let tokens = self.invites.own.get(&account_id).cloned();
        let joining = self.invites.joining.iter().position(|j| j.account_id == account_id && j.invite.addr == addr);
        match step.step.as_str() {
            // Someone opened this account's link.
            "vc-request" => {
                if tokens.is_some_and(|t| step.invitenumber.as_deref() == Some(t.invitenumber.as_str())) && self.pgp_key_of(&addr).is_some() {
                    self.send_step(&account_id, from, Handshake { step: "vc-auth-required".into(), ..Handshake::default() });
                }
            }
            // The inviter answered the request for a link this account opened.
            "vc-auth-required" => {
                let Some(j) = joining.map(|i| self.invites.joining[i].clone()) else { return };
                if !encrypted
                    || !shows_me
                    || signer.as_deref() != Some(j.invite.fingerprint.as_str())
                    || !self.keep_checked_key(&addr, &j.invite.fingerprint, carried)
                {
                    return;
                }
                let next = Handshake {
                    step: "vc-request-with-auth".into(),
                    auth: Some(j.invite.auth),
                    fingerprint: Some(own.fingerprint),
                    ..Handshake::default()
                };
                self.send_step(&account_id, from, next);
            }
            // The one who opened this account's link proves it with the second token.
            "vc-request-with-auth" => {
                let Some(fingerprint) = step.fingerprint.filter(|fp| signer.as_deref() == Some(fp.as_str())) else { return };
                if !encrypted || !shows_me || !tokens.is_some_and(|t| step.auth.as_deref() == Some(t.auth.as_str())) {
                    return;
                }
                if self.keep_checked_key(&addr, &fingerprint, carried) {
                    self.invites.verified.insert(addr, fingerprint);
                    self.send_step(&account_id, from, Handshake { step: "vc-contact-confirm".into(), ..Handshake::default() });
                }
            }
            // The inviter confirms: the chat is verified.
            "vc-contact-confirm" => {
                let Some(i) = joining else { return };
                let fingerprint = self.invites.joining[i].invite.fingerprint.clone();
                if encrypted && signer.as_deref() == Some(fingerprint.as_str()) {
                    self.invites.joining.remove(i);
                    self.invites.verified.insert(addr, fingerprint);
                }
            }
            _ => {}
        }
        self.save_invites();
    }

    /// Makes sure the key kept for `addr` is the one with `fingerprint`, from the message's
    /// own `Autocrypt` header when Merel had another (or none): a key checked against an
    /// invitation outweighs one taken on trust.
    fn keep_checked_key(&mut self, addr: &str, fingerprint: &str, carried: Option<&Carried>) -> bool {
        if self.pgp_key_of(addr).is_some_and(|k| k.fingerprint == fingerprint) {
            return true;
        }
        match carried.and_then(|c| c.autocrypt.clone()).filter(|(a, k)| a == addr && k.info().fingerprint == fingerprint) {
            Some((a, key)) => {
                self.replace_key(a, key);
                true
            }
            None => false,
        }
    }

    fn save_invites(&self) {
        if let Some(path) = self.config_path.as_deref()
            && let Err(e) = config::save_json(&config::invites_path(path), &self.invites)
        {
            log::warn!("Invitations not saved: {e}");
        }
    }

    pub(crate) fn load_invites(&mut self) {
        if let Some(path) = self.config_path.as_deref() {
            self.invites = config::load_json(&config::invites_path(path));
        }
    }
}

/// The fingerprint of the key the opened message is signed with, when the signature holds:
/// a key Merel knew, or the one the message brought along.
fn signed_by(body: &Body, carried: Option<&Carried>) -> Option<String> {
    match &body.pgp_signature {
        Some(merel_pgp::Verdict::Valid { signer }) => Some(signer.fingerprint.clone()),
        // Valid with the key it brought, while Merel keeps another for the address.
        Some(merel_pgp::Verdict::UnknownKey(fp))
            if !fp.is_empty() && carried.and_then(|c| c.autocrypt.as_ref()).is_some_and(|(_, k)| &k.info().fingerprint == fp) =>
        {
            Some(fp.clone())
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use merel_model::{Account, Auth, Folder, Server, Tls};
    use merel_secrets::Secret;

    use super::*;
    use crate::Job;

    pub(crate) fn account(id: &str, name: &str, email: &str) -> Account {
        Account {
            id: id.into(),
            name: name.into(),
            address: Address::new(Some(name), email).unwrap(),
            username: email.into(),
            protocol: merel_model::Protocol::ImapSmtp,
            auth: Auth::Password,
            incoming: Some(Server { host: "127.0.0.1".into(), port: 1, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        }
    }

    pub(crate) fn engine(a: &Account) -> Engine {
        let mut e = Engine::sample();
        e.add_account(a.clone(), Secret::new("x")).unwrap();
        e.session.folders.push(Folder {
            id: format!("{}/INBOX", a.id),
            account_id: a.id.clone(),
            name: "Postvak IN".into(),
            path: "INBOX".into(),
            kind: FolderKind::Inbox,
            unread: 0,
            parent: None,
        });
        e
    }

    /// What `e` sends next from `a`, as it goes on the wire.
    pub(crate) fn sent(e: &mut Engine, a: &Account) -> Vec<Vec<u8>> {
        e.session
            .take_jobs()
            .into_iter()
            .filter_map(|j| match j {
                Job::Send { draft, .. } => {
                    let raw = merel_smtp::build(&a.address, &draft).unwrap().formatted();
                    Some(crate::certificates::outgoing(&e.keys, a, &draft, raw).unwrap())
                }
                _ => None,
            })
            .collect()
    }

    /// Delivers `raw` into the inbox of `e`'s account `a` as message `n`, and lets `e` read it
    /// as the worker would hand it over.
    pub(crate) fn deliver(e: &mut Engine, a: &Account, n: u32, raw: &[u8]) {
        let folder = format!("{}/INBOX", a.id);
        let header = merel_mime::parse_header(raw);
        let listed = merel_mailbox::RemoteHeader {
            id: n.to_string(),
            seen: false,
            flagged: false,
            answered: false,
            size: raw.len() as u32,
            has_attachments: false,
            date: header.date.unwrap_or(0),
            header,
            categories: Vec::new(),
        };
        e.session.messages.push(crate::message_of(&folder, listed).unwrap());
        let id = format!("{folder}#{n}");
        let (body, header, carried) = crate::certificates::read(&e.keys, &a.id, raw);
        let mut event =
            crate::Event::Body { message_id: id, body: Box::new(body), header: Box::new(header), raw: raw.to_vec(), carried: carried.map(Box::new) };
        e.take_in(&mut event);
        e.session.apply(event);
    }

    #[test]
    fn reads_delta_chat_links_and_qr_codes() {
        let fp = "1234567890ABCDEF1234567890ABCDEF12345678";
        let link = format!("https://i.delta.chat/#{fp}&v=3&i=inv-1_X&s=auth2&a=alice%40example.org&n=Alice+de+Vries");
        let invite = parse_invite(&link).unwrap();
        assert_eq!(
            invite,
            Invite {
                fingerprint: fp.into(),
                addr: "alice@example.org".into(),
                name: "Alice de Vries".into(),
                invitenumber: "inv-1_X".into(),
                auth: "auth2".into()
            }
        );
        assert_eq!(parse_invite(&invite.link()).unwrap(), invite, "Merel's own links read back");
        assert!(invite.link().starts_with(&format!("https://i.delta.chat/#{fp}&i=")), "{}", invite.link());
        let qr = format!("OPENPGP4FPR:{}#a=alice%40example.org&n=Alice&i=inv&s=auth", fp.to_lowercase());
        assert_eq!(parse_invite(&qr).unwrap().fingerprint, fp);
        assert!(parse_invite(&format!("{link}&x=group1&g=Fietsclub")).unwrap_err().contains("groep"));
        for wrong in ["https://example.org/#x", "https://i.delta.chat/#123&a=a@b.nl&i=x&s=y", &link.replace("&s=auth2", "&s=a;b")] {
            assert!(parse_invite(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn a_first_message_waits_for_the_key_while_an_invitation_goes_out() {
        let alain = account("a1", "Alain Rees", "alain@voorbeeld.nl");
        let bob = account("b1", "Bob", "bob@example.org");
        let (mut a, mut b) = (engine(&alain), engine(&bob));
        a.enable_chat("a1").unwrap();
        b.enable_chat("b1").unwrap();
        let draft = crate::chat::new_chat(std::slice::from_ref(&bob.address), "", "Zullen we om tien uur beginnen?").unwrap();
        assert_eq!(a.hold_or_send("a1", draft.clone()).unwrap(), None, "held back");
        assert_eq!(a.waiting().len(), 1);
        assert_eq!(a.waiting()[0].chat_id(), crate::chat::person_chat_id("a1", "bob@example.org"));

        // Only the invitation goes out: unencrypted, a chat message with the link, and not a
        // word of the message itself.
        let out = sent(&mut a, &alain);
        assert_eq!(out.len(), 1);
        let text = String::from_utf8_lossy(&out[0]).into_owned();
        assert!(!merel_pgp::is_encrypted(&out[0]) && text.contains("Chat-Version: 1.0") && text.contains("i.delta.chat"), "{text}");
        assert!(!text.contains("tien uur") && text.contains("Autocrypt:"), "{text}");
        // A second message to Bob the same week sends no second invitation.
        let again = crate::chat::new_chat(std::slice::from_ref(&bob.address), "", "En de begroting?").unwrap();
        assert_eq!(a.hold_or_send("a1", again).unwrap(), None);
        assert!(sent(&mut a, &alain).is_empty());

        // Bob answers from Delta Chat, encrypted to the key the invitation brought: now Alain has
        // his key, and both messages go out, encrypted.
        let mut b_in = out[0].clone();
        deliver(&mut b, &bob, 1, &std::mem::take(&mut b_in));
        let reply = crate::chat::new_chat(std::slice::from_ref(&alain.address), "", "Prima!").unwrap();
        assert!(b.hold_or_send("b1", reply).unwrap().is_some(), "Bob has Alain's key from the invitation");
        let answer = sent(&mut b, &bob).remove(0);
        assert!(merel_pgp::is_encrypted(&answer));
        deliver(&mut a, &alain, 1, &answer);
        assert!(a.waiting().is_empty());
        let out = sent(&mut a, &alain);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|raw| merel_pgp::is_encrypted(raw)));
    }

    #[test]
    fn an_invitation_ends_in_a_verified_chat_on_both_sides() {
        let alain = account("a1", "Alain Rees", "alain@voorbeeld.nl");
        let bob = account("b1", "Bob", "bob@example.org");
        let (mut a, mut b) = (engine(&alain), engine(&bob));
        let link = a.invite_link("a1").unwrap();
        assert_eq!(a.invite_link("a1").unwrap(), link, "the link stays the same");
        assert!(b.join("b1", "geen link").is_err());
        let chat = b.join("b1", &link).unwrap();
        assert_eq!(chat, crate::chat::person_chat_id("b1", "alain@voorbeeld.nl"));
        assert_eq!(b.pending_invites().len(), 1);

        // 1. Bob asks, unencrypted, with his key and the first token only.
        let request = sent(&mut b, &bob).remove(0);
        let text = String::from_utf8_lossy(&request).into_owned();
        assert!(text.contains("Secure-Join: vc-request") && !merel_pgp::is_encrypted(&request), "{text}");
        let auth = link.split("&s=").nth(1).unwrap().split('&').next().unwrap();
        assert!(!text.contains(auth), "the second token stays home");
        deliver(&mut a, &alain, 1, &request);
        // 2. Alain answers, encrypted.
        let answer = sent(&mut a, &alain).remove(0);
        assert!(merel_pgp::is_encrypted(&answer));
        deliver(&mut a, &alain, 1, &request);
        assert!(sent(&mut a, &alain).is_empty(), "a step is answered once");
        deliver(&mut b, &bob, 1, &answer);
        // 3. Bob proves he has the link, encrypted.
        let proof = sent(&mut b, &bob).remove(0);
        assert!(merel_pgp::is_encrypted(&proof) && !String::from_utf8_lossy(&proof).contains(auth));
        deliver(&mut a, &alain, 2, &proof);
        assert!(a.is_verified("bob@example.org"));
        // 4. Alain confirms.
        let confirm = sent(&mut a, &alain).remove(0);
        deliver(&mut b, &bob, 2, &confirm);
        assert!(b.is_verified("alain@voorbeeld.nl") && b.pending_invites().is_empty());
        assert!(a.session.chats().is_empty() && b.session.chats().is_empty(), "the handshake shows no messages");
        assert!(a.session.visible_messages().is_empty() || a.session.visible_messages().iter().all(|m| m.chat.is_none()));

        // Now they can chat.
        let hello = crate::chat::new_chat(&[Address::new(None, "bob@example.org").unwrap()], "", "Hoi Bob").unwrap();
        assert!(a.send_chat("a1", hello).is_ok());
    }

    #[test]
    fn a_wrong_token_or_a_stranger_gets_no_answer() {
        let alain = account("a1", "Alain Rees", "alain@voorbeeld.nl");
        let bob = account("b1", "Bob", "bob@example.org");
        let (mut a, mut b) = (engine(&alain), engine(&bob));
        let link = a.invite_link("a1").unwrap();
        // A link with the right fingerprint but made-up tokens.
        let invite = parse_invite(&link).unwrap();
        let forged = Invite { invitenumber: "geraden".into(), auth: "geraden".into(), ..invite.clone() }.link();
        b.join("b1", &forged).unwrap();
        let request = sent(&mut b, &bob).remove(0);
        deliver(&mut a, &alain, 1, &request);
        assert!(sent(&mut a, &alain).is_empty());

        // A renewed link makes the old one stop working.
        let renewed = a.renew_invite_link("a1").unwrap();
        assert_ne!(renewed, link);
        b.join("b1", &link).unwrap();
        let request = sent(&mut b, &bob).remove(0);
        deliver(&mut a, &alain, 2, &request);
        assert!(sent(&mut a, &alain).is_empty());
        let own = b.invite_link("b1").unwrap();
        assert!(b.join("b1", &own).is_err(), "not one's own");
    }
}
