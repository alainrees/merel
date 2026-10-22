//! Merel's own meeting room, between Merel users and without a server: picture and sound go
//! straight between the computers (WebRTC, encrypted end to end with DTLS-SRTP), and the few
//! steps that let them find each other travel as encrypted, signed chat messages
//! ([`CallSignal`]). Because those steps carry each side's DTLS fingerprint, the media is tied
//! to the OpenPGP keys of the chat.
//!
//! The engine keeps track of who is in which room and says what the media side must do
//! ([`CallTask`]); the media side makes the offers and answers and hands them back. In a group
//! every pair connects once: the one whose address sorts first makes the offer, the other
//! answers.

use std::collections::BTreeSet;

use merel_mime::{Body, Encryption, ParsedHeader};
use merel_model::{Address, CallSignal, CallStep, ChatTag, Draft, FolderKind};

use crate::chat::{CHAT_SUBJECT, Chat};
use crate::{Engine, SAMPLE_ACCOUNT};

/// How long a ring, a join or a leave counts after it was sent; older ones (mail fetched
/// later, a computer that was off) no longer ring.
const FRESH: i64 = 3 * 3600;
/// How long an offer or an answer counts after it was sent.
const FRESH_SDP: i64 = 30 * 60;
/// A room nobody signalled in for this long is forgotten.
const STALE: i64 = 12 * 3600;
/// Most rooms kept at once.
const MAX_CALLS: usize = 20;
/// Most message ids remembered as handled.
const MAX_HANDLED: usize = 500;

/// A meeting in Merel's own room that this computer knows of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub room: String,
    pub account_id: String,
    /// The chat it belongs to.
    pub chat_id: String,
    /// The group tag its steps carry; `None` between two people.
    pub group: Option<ChatTag>,
    /// Everyone in the chat but the account itself.
    pub people: Vec<Address>,
    /// Who started it, lower case; the account's own address when it was this computer.
    pub started_by: String,
    /// Whether this computer is in the room.
    pub joined: bool,
    /// The others known to be in the room, lower case.
    pub present: BTreeSet<String>,
    /// When something last happened in it, in seconds since the Unix epoch.
    pub last: i64,
}

/// What the media side must do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallTask {
    /// Make an offer for `peer` and hand it to [`Engine::send_call_offer`].
    Offer { room: String, peer: String },
    /// Answer `peer`'s offer and hand the answer to [`Engine::send_call_answer`].
    Answer { room: String, peer: String, sdp: String },
    /// `peer` answered this computer's offer.
    Accept { room: String, peer: String, sdp: String },
    /// Close the connection with `peer`.
    Close { room: String, peer: String },
}

/// The rooms and what waits for the media side; kept in memory only.
#[derive(Debug, Default)]
pub struct Calls {
    pub calls: Vec<Call>,
    tasks: Vec<CallTask>,
    handled: Vec<String>,
}

/// Whether `me` makes the offer to `peer`: the address that sorts first does.
fn offers_to(me: &str, peer: &str) -> bool {
    me < peer
}

impl Engine {
    /// The meetings this computer knows of, the latest first.
    pub fn calls(&self) -> Vec<&Call> {
        let mut out: Vec<&Call> = self.calls.calls.iter().collect();
        out.sort_by_key(|c| std::cmp::Reverse(c.last));
        out
    }

    pub fn call(&self, room: &str) -> Option<&Call> {
        self.calls.calls.iter().find(|c| c.room == room)
    }

    /// What the media side must do, in order; each task is handed out once.
    pub fn take_call_tasks(&mut self) -> Vec<CallTask> {
        std::mem::take(&mut self.calls.tasks)
    }

    /// Starts a meeting in the chat `chat_id` and rings everyone in it. Only for people whose
    /// key Merel has: the steps never travel unencrypted. Returns the room.
    pub fn start_call(&mut self, chat_id: &str) -> Result<String, String> {
        let chat = self.session.chats().into_iter().find(|c| c.id == chat_id).ok_or_else(|| merel_i18n::tr("call-no-chat"))?;
        let missing: Vec<String> = chat.people.iter().filter(|p| self.pgp_key_of(p.email()).is_none()).map(|p| p.display().to_string()).collect();
        if !missing.is_empty() {
            return Err(merel_i18n::tr_args("call-no-key", &[("names", missing.join(", ").into())]));
        }
        let me = self.own_address(&chat.account_id).ok_or_else(|| merel_i18n::tr("account-gone"))?;
        let mut random = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
        let room = ChatTag::new_group_id(random);
        self.forget_stale();
        self.calls.calls.push(Call {
            room: room.clone(),
            account_id: chat.account_id.clone(),
            chat_id: chat.id.clone(),
            group: chat.group.clone(),
            people: chat.people.clone(),
            started_by: me,
            joined: true,
            present: BTreeSet::new(),
            last: crate::drafts::now(),
        });
        self.signal_all(&room, CallStep::Ring);
        Ok(room)
    }

    /// Joins the meeting `room`: tells the others, and offers a connection to those already in
    /// it whom this computer connects to first.
    pub fn join_call(&mut self, room: &str) -> Result<(), String> {
        let call = self.calls.calls.iter_mut().find(|c| c.room == room).ok_or_else(|| merel_i18n::tr("call-gone"))?;
        if call.joined {
            return Ok(());
        }
        call.joined = true;
        call.last = crate::drafts::now();
        let (account_id, present) = (call.account_id.clone(), call.present.clone());
        let me = self.own_address(&account_id).unwrap_or_default();
        for peer in present.iter().filter(|p| offers_to(&me, p)) {
            self.calls.tasks.push(CallTask::Offer { room: room.to_string(), peer: peer.clone() });
        }
        self.signal_all(room, CallStep::Join);
        Ok(())
    }

    /// Leaves the meeting `room`: tells the others and closes every connection.
    pub fn leave_call(&mut self, room: &str) {
        let Some(call) = self.calls.calls.iter_mut().find(|c| c.room == room) else { return };
        if !call.joined {
            return;
        }
        call.joined = false;
        let present = call.present.clone();
        for peer in present {
            self.calls.tasks.push(CallTask::Close { room: room.to_string(), peer });
        }
        self.signal_all(room, CallStep::Leave);
    }

    /// Sends the offer the media side made for `peer`.
    pub fn send_call_offer(&mut self, room: &str, peer: &str, sdp: String) -> Result<(), String> {
        self.signal_one(room, peer, CallStep::Offer, sdp)
    }

    /// Sends the media side's answer to `peer`'s offer.
    pub fn send_call_answer(&mut self, room: &str, peer: &str, sdp: String) -> Result<(), String> {
        self.signal_one(room, peer, CallStep::Answer, sdp)
    }

    fn own_address(&self, account_id: &str) -> Option<String> {
        self.session.accounts.iter().find(|a| a.id == account_id).map(|a| a.address.email().to_lowercase())
    }

    fn signal_one(&mut self, room: &str, peer: &str, step: CallStep, sdp: String) -> Result<(), String> {
        let call = self.call(room).filter(|c| c.joined).ok_or_else(|| merel_i18n::tr("call-gone"))?;
        let to = call.people.iter().find(|p| p.email().eq_ignore_ascii_case(peer)).cloned().ok_or_else(|| merel_i18n::tr("call-gone"))?;
        let draft = signal_draft(call, vec![to], CallSignal { room: room.to_string(), step, sdp: Some(sdp) });
        let account_id = call.account_id.clone();
        self.session.send(&account_id, draft);
        Ok(())
    }

    fn signal_all(&mut self, room: &str, step: CallStep) {
        let Some(call) = self.call(room) else { return };
        let draft = signal_draft(call, call.people.clone(), CallSignal { room: room.to_string(), step, sdp: None });
        let account_id = call.account_id.clone();
        self.session.send(&account_id, draft);
    }

    fn forget_stale(&mut self) {
        let now = crate::drafts::now();
        self.calls.calls.retain(|c| c.joined || now - c.last < STALE);
        while self.calls.calls.len() >= MAX_CALLS {
            let oldest = self.calls.calls.iter().enumerate().min_by_key(|(_, c)| c.last).map(|(i, _)| i).unwrap_or(0);
            self.calls.calls.remove(oldest);
        }
    }

    /// Takes in the meeting step in the message `message_id` that just came in, opened. Only
    /// a step from the inbox, encrypted and signed with the key Merel keeps for its sender, by
    /// someone in the chat, counts.
    pub(crate) fn handle_call(&mut self, message_id: &str, header: &ParsedHeader, body: &Body) {
        let Some(tag) = header.chat.as_ref() else { return };
        let Some(signal) = tag.call.clone() else { return };
        let Some(m) = self.session.message(message_id) else { return };
        let Some(folder) = self.session.folder(&m.folder_id).filter(|f| f.kind == FolderKind::Inbox) else { return };
        let account_id = folder.account_id.clone();
        if account_id == SAMPLE_ACCOUNT || self.pgp_key(&account_id).is_none() {
            return;
        }
        let Some(me) = self.own_address(&account_id) else { return };
        let from = m.from.email().to_lowercase();
        let date = m.date;
        if from == me {
            return;
        }
        let seen = m.message_id.clone().unwrap_or_else(|| message_id.to_string());
        if self.calls.handled.contains(&seen) {
            return;
        }
        self.calls.handled.push(seen);
        if self.calls.handled.len() > MAX_HANDLED {
            self.calls.handled.remove(0);
        }
        let signer = match &body.pgp_signature {
            Some(merel_pgp::Verdict::Valid { signer }) => Some(signer.fingerprint.clone()),
            _ => None,
        };
        let known = self.pgp_key_of(&from).map(|k| k.fingerprint);
        if body.encrypted != Some(Encryption::Opened) || signer.is_none() || signer != known {
            return;
        }
        let now = crate::drafts::now();
        let fresh = if matches!(signal.step, CallStep::Offer | CallStep::Answer) { FRESH_SDP } else { FRESH };
        if now - date > fresh {
            return;
        }
        let chat_id = match &tag.group_id {
            Some(g) => crate::chat::group_chat_id(&account_id, g),
            None => crate::chat::person_chat_id(&account_id, &from),
        };
        let room = signal.room.clone();
        let exists = self.calls.calls.iter().any(|c| c.room == room);
        if !exists {
            // Only a ring or a join makes a room known; the chat must hold the sender.
            if !matches!(signal.step, CallStep::Ring | CallStep::Join) {
                return;
            }
            let chats = self.session.chats();
            let chat: Option<&Chat> = chats.iter().find(|c| c.id == chat_id);
            let people = match chat {
                Some(c) => c.people.clone(),
                None if tag.group_id.is_none() => vec![m.from.clone()],
                None => return,
            };
            if !people.iter().any(|p| p.email().eq_ignore_ascii_case(&from)) {
                return;
            }
            self.forget_stale();
            self.calls.calls.push(Call {
                room: room.clone(),
                account_id: account_id.clone(),
                chat_id: chat_id.clone(),
                group: tag.group_id.as_ref().map(|_| ChatTag { call: None, handshake: None, invite: false, ..tag.clone() }),
                people,
                started_by: from.clone(),
                joined: false,
                present: BTreeSet::new(),
                last: now,
            });
        }
        let Some(call) = self.calls.calls.iter_mut().find(|c| c.room == room) else { return };
        if call.chat_id != chat_id || !call.people.iter().any(|p| p.email().eq_ignore_ascii_case(&from)) {
            return;
        }
        call.last = now;
        let joined = call.joined;
        match signal.step {
            CallStep::Ring | CallStep::Join => {
                let new = call.present.insert(from.clone());
                if new && joined && offers_to(&me, &from) {
                    self.calls.tasks.push(CallTask::Offer { room, peer: from });
                }
            }
            CallStep::Offer => {
                call.present.insert(from.clone());
                if joined && !offers_to(&me, &from) {
                    let sdp = signal.sdp.unwrap_or_default();
                    self.calls.tasks.push(CallTask::Answer { room, peer: from, sdp });
                }
            }
            CallStep::Answer => {
                if joined && offers_to(&me, &from) {
                    let sdp = signal.sdp.unwrap_or_default();
                    self.calls.tasks.push(CallTask::Accept { room, peer: from, sdp });
                }
            }
            CallStep::Leave => {
                if call.present.remove(&from) && joined {
                    self.calls.tasks.push(CallTask::Close { room, peer: from });
                }
            }
        }
    }
}

/// A meeting step as a chat message of the call's chat: encrypted, and with a line of text
/// for whoever reads it without Merel (Delta Chat shows the ring).
fn signal_draft(call: &Call, to: Vec<Address>, signal: CallSignal) -> Draft {
    let body = match signal.step {
        CallStep::Ring => merel_i18n::tr("call-ring-text"),
        _ => String::new(),
    };
    let tag = ChatTag { call: Some(signal), ..call.group.clone().unwrap_or_default() };
    Draft { to, subject: CHAT_SUBJECT.into(), body, sign: Some(true), encrypt: true, chat: Some(tag), ..Draft::default() }
}

#[cfg(test)]
mod tests {
    use merel_model::Account;

    use super::*;
    use crate::securejoin::tests::{account, deliver, engine, sent};

    /// Two engines that know each other's keys, with a chat between them.
    fn pair() -> (Engine, Account, Engine, Account) {
        let alain = account("a1", "Alain Rees", "alain@voorbeeld.nl");
        let bob = account("b1", "Bob", "bob@example.org");
        let (mut a, mut b) = (engine(&alain), engine(&bob));
        a.enable_chat("a1").unwrap();
        b.enable_chat("b1").unwrap();
        let a_pub = a.keys.openpgp.own.read().unwrap()["a1"].public();
        let b_pub = b.keys.openpgp.own.read().unwrap()["b1"].public();
        a.keys.openpgp.people.write().unwrap().insert("bob@example.org".into(), b_pub);
        b.keys.openpgp.people.write().unwrap().insert("alain@voorbeeld.nl".into(), a_pub);
        // A chat message each way, so both have the chat.
        a.hold_or_send("a1", crate::chat::new_chat(std::slice::from_ref(&bob.address), "", "Hoi").unwrap()).unwrap();
        let hoi = sent(&mut a, &alain).remove(0);
        deliver(&mut b, &bob, 1, &hoi);
        (a, alain, b, bob)
    }

    #[test]
    fn a_ring_and_an_offer_and_answer_travel_encrypted_and_connect_the_pair() {
        let (mut a, alain, mut b, bob) = pair();
        let chat = crate::chat::person_chat_id("b1", "alain@voorbeeld.nl");
        assert!(b.session.chats().iter().any(|c| c.id == chat));

        // Bob rings; the ring is encrypted and its steps stay inside.
        let room = b.start_call(&chat).unwrap();
        let ring = sent(&mut b, &bob).remove(0);
        assert!(merel_pgp::is_encrypted(&ring));
        assert!(!String::from_utf8_lossy(&ring).contains(&room), "the room stays inside the encryption");
        deliver(&mut a, &alain, 2, &ring);
        let call = a.call(&room).expect("Alain sees the meeting");
        assert!(!call.joined && call.present.contains("bob@example.org") && call.started_by == "bob@example.org");
        assert_eq!(a.session.chats()[0].messages.len(), 1, "the ring shows in the chat");

        // Alain joins; "alain" sorts before "bob", so Alain offers.
        a.join_call(&room).unwrap();
        assert_eq!(a.take_call_tasks(), [CallTask::Offer { room: room.clone(), peer: "bob@example.org".into() }]);
        let join = sent(&mut a, &alain).remove(0);
        deliver(&mut b, &bob, 2, &join);
        assert!(b.take_call_tasks().is_empty(), "Bob waits for the offer");
        a.send_call_offer(&room, "bob@example.org", "v=0\r\no=alain\r\n".into()).unwrap();
        let offer = sent(&mut a, &alain).remove(0);
        assert!(merel_pgp::is_encrypted(&offer) && !String::from_utf8_lossy(&offer).contains("Merel-Call"));
        deliver(&mut b, &bob, 3, &offer);
        assert_eq!(
            b.take_call_tasks(),
            [CallTask::Answer { room: room.clone(), peer: "alain@voorbeeld.nl".into(), sdp: "v=0\r\no=alain\r\n".into() }]
        );
        b.send_call_answer(&room, "alain@voorbeeld.nl", "v=0\r\no=bob\r\n".into()).unwrap();
        deliver(&mut a, &alain, 3, &sent(&mut b, &bob).remove(0));
        assert_eq!(a.take_call_tasks(), [CallTask::Accept { room: room.clone(), peer: "bob@example.org".into(), sdp: "v=0\r\no=bob\r\n".into() }]);
        // The steps after the ring are no chat messages.
        assert_eq!(a.session.chats()[0].messages.len(), 1);

        // Bob leaves: Alain closes the connection.
        b.leave_call(&room);
        assert_eq!(b.take_call_tasks(), [CallTask::Close { room: room.clone(), peer: "alain@voorbeeld.nl".into() }]);
        deliver(&mut a, &alain, 4, &sent(&mut b, &bob).remove(0));
        assert_eq!(a.take_call_tasks(), [CallTask::Close { room: room.clone(), peer: "bob@example.org".into() }]);
        assert!(a.call(&room).unwrap().present.is_empty());
    }

    #[test]
    fn a_step_from_a_stranger_or_without_a_key_does_nothing() {
        let (mut a, alain, _b, _bob) = pair();
        // Mallory has no key with Alain: no meeting can start with her, and her unencrypted
        // ring is ignored.
        let mallory = Address::new(Some("Mallory"), "mallory@example.org").unwrap();
        let draft = crate::chat::new_chat(std::slice::from_ref(&mallory), "", "x").unwrap();
        let _ = a.hold_or_send("a1", draft);
        let _ = sent(&mut a, &alain);
        let fake = b"From: Mallory <mallory@example.org>\r\nTo: alain@voorbeeld.nl\r\nSubject: [...]\r\nChat-Version: 1.0\r\n\
                     Merel-Call: ring; room=R1\r\nMessage-ID: <m1@example.org>\r\nContent-Type: text/plain\r\n\r\nBel me\r\n";
        deliver(&mut a, &alain, 9, fake);
        assert!(a.call("R1").is_none(), "unencrypted and unsigned: ignored");
        let chat = crate::chat::person_chat_id("a1", "mallory@example.org");
        assert!(a.start_call(&chat).is_err());
    }
}
