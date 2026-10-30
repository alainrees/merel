//! Chat over e-mail, the way Delta Chat does it (chatmail): a chat message is an ordinary,
//! end-to-end encrypted message with a `Chat-Version` header, so any mail account carries it
//! and the other side needs no Merel. A group chat names itself with `Chat-Group-ID` and
//! `Chat-Group-Name`.

use serde::{Deserialize, Serialize};

use crate::clean_text;

/// The `Chat-Version` Merel writes.
pub const CHAT_VERSION: &str = "1.0";
/// Most characters a group id may have; Delta Chat makes them 11 to 22.
pub const MAX_GROUP_ID: usize = 64;
/// Most characters a group name keeps.
pub const MAX_GROUP_NAME: usize = 200;

/// What makes a message a chat message, from its headers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatTag {
    /// The group the message belongs to; `None` for a chat between two people.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
    /// Set on a step of an invitation's handshake, which is no message to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake: Option<Handshake>,
    /// Set on the invitation Merel mails by itself to someone without a key, so a chat can
    /// start: it goes out unencrypted and holds nothing but the invitation link, never the
    /// message that waits for the key. Not a header; the other side reads it as a chat message.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invite: bool,
    /// Set on a step of setting up a meeting between Merel users (`Merel-Call`), which only
    /// travels encrypted. Only a [`CallStep::Ring`] is a message to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<CallSignal>,
    /// Set on a reaction going out (`Merel-Reaction`): the Message-ID of the message reacted
    /// to. The text is the emoji; an empty text takes the reaction back. Only Signal sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction_to: Option<String>,
    /// The emoji people reacted with (`Merel-Reactions`), one per person. Only a Signal
    /// account's messages carry them; the engine drops them from any other.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reacted>,
    /// The message this one answers, quoted above it (`Merel-Quote`). Going out, its
    /// Message-ID says which message; coming in, its name and text show. Only Signal sends
    /// and shows it; the engine drops it from any other account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<Quoted>,
    /// For one's own message: whether it arrived or was read (`Merel-Delivery`), as Signal's
    /// receipts say. Only Signal says it; the engine drops it from any other account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
    /// Set on an edit going out (`Merel-Edit`): the Message-ID of one's own message whose
    /// text this one's text replaces. Only Signal sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_of: Option<String>,
    /// Set on a deletion going out (`Merel-Delete`): the Message-ID of one's own message to
    /// delete for everyone. Only Signal sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_of: Option<String>,
    /// Whether the writer changed the text after sending it (`Merel-Edited`). Only Signal says
    /// it; the engine drops it from any other account.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub edited: bool,
    /// Whether the writer deleted the message for everyone (`Merel-Deleted`): what is left is
    /// a line saying so. Only Signal says it; the engine drops it from any other account.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// Whether it is a voice message (`Merel-Voice`): its audio is to listen to, not a file.
    /// Only Signal says it; the engine drops it from any other account.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub voice: bool,
}

/// How far a message one sent came.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delivery {
    Delivered,
    Read,
}

impl Delivery {
    /// From a `Merel-Delivery` header value.
    pub fn from_header(value: Option<&str>) -> Option<Delivery> {
        match value?.trim() {
            "delivered" => Some(Delivery::Delivered),
            "read" => Some(Delivery::Read),
            _ => None,
        }
    }
}

/// A message quoted in the answer to it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quoted {
    /// Its Message-ID, as Merel lists it; empty when the message is not known.
    #[serde(default)]
    pub message_id: String,
    /// Who wrote it, by name.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub text: String,
}

/// Most characters of a quoted message kept.
pub const MAX_QUOTE: usize = 500;

impl Quoted {
    /// The `Merel-Quote` header value: base64 JSON.
    pub fn header(&self) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(self).unwrap_or_default())
    }

    /// The quote a `Merel-Quote` header value holds, if it is one.
    pub fn from_header(value: Option<&str>) -> Option<Quoted> {
        use base64::Engine as _;
        let b64: String = value?.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        let q: Quoted = serde_json::from_slice(&bytes).ok()?;
        let message_id = clean_text(q.message_id.trim()).chars().take(998).collect();
        let name = clean_text(q.name.trim()).chars().take(MAX_GROUP_NAME).collect();
        // Line breaks stay: the quote shows its first lines.
        let text = q.text.chars().filter(|c| *c == '\n' || !c.is_control()).take(MAX_QUOTE).collect();
        Some(Quoted { message_id, name, text })
    }
}

/// One person's reaction to a message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reacted {
    pub emoji: String,
    /// Who reacted, by name.
    pub name: String,
    /// Whether it was the account itself.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mine: bool,
}

/// Most reactions one message keeps.
pub const MAX_REACTIONS: usize = 200;
/// Most bytes one reaction's emoji may have: a family emoji with skin tones is about 40.
pub const MAX_EMOJI: usize = 64;

impl Reacted {
    /// The `Merel-Reactions` header value: the reactions as base64 JSON.
    pub fn header(reactions: &[Reacted]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(reactions).unwrap_or_default())
    }

    /// The reactions a `Merel-Reactions` header value holds; none when it is not one.
    pub fn from_header(value: Option<&str>) -> Vec<Reacted> {
        use base64::Engine as _;
        let Some(value) = value else { return Vec::new() };
        let b64: String = value.chars().filter(|c| !c.is_whitespace()).collect();
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { return Vec::new() };
        let all: Vec<Reacted> = serde_json::from_slice(&bytes).unwrap_or_default();
        all.into_iter()
            .take(MAX_REACTIONS)
            .filter_map(|r| {
                let emoji = clean_text(r.emoji.trim());
                let name = clean_text(r.name.trim()).chars().take(MAX_GROUP_NAME).collect();
                (!emoji.is_empty() && emoji.len() <= MAX_EMOJI).then_some(Reacted { emoji, name, mine: r.mine })
            })
            .collect()
    }

    /// The emoji used, each once with how many chose it, in the order they first came.
    pub fn tally(reactions: &[Reacted]) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        for r in reactions {
            match out.iter_mut().find(|(e, _)| *e == r.emoji) {
                Some((_, n)) => *n += 1,
                None => out.push((r.emoji.clone(), 1)),
            }
        }
        out
    }
}

/// Most bytes of a session description a call step carries.
pub const MAX_SDP: usize = 64 * 1024;

/// A step of setting up a meeting in Merel's own meeting room: picture and sound go straight
/// between the computers (WebRTC), and these steps, sent as encrypted chat messages, are how
/// they find each other. In a group every pair connects once: the one whose address sorts
/// first makes the offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSignal {
    /// The meeting room, a token Merel makes when the meeting starts.
    pub room: String,
    pub step: CallStep,
    /// The session description of an offer or an answer (SDP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdp: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallStep {
    /// Someone started a meeting and asks the chat to join; the sender is in it.
    Ring,
    /// The sender joined.
    Join,
    /// To one person: the sender's connection offer.
    Offer,
    /// To one person: the answer to their offer.
    Answer,
    /// The sender left.
    Leave,
}

impl CallStep {
    pub fn name(self) -> &'static str {
        match self {
            CallStep::Ring => "ring",
            CallStep::Join => "join",
            CallStep::Offer => "offer",
            CallStep::Answer => "answer",
            CallStep::Leave => "leave",
        }
    }

    fn from_name(name: &str) -> Option<CallStep> {
        [CallStep::Ring, CallStep::Join, CallStep::Offer, CallStep::Answer, CallStep::Leave].into_iter().find(|s| s.name() == name)
    }
}

impl CallSignal {
    /// The `Merel-Call` header value: `offer; room=Ab-9_x`.
    pub fn header(&self) -> String {
        format!("{}; room={}", self.step.name(), self.room)
    }

    /// The `Merel-Call-Sdp` header value: the session description in base64, so its lines
    /// survive as one header.
    pub fn sdp_header(&self) -> Option<String> {
        use base64::Engine as _;
        self.sdp.as_ref().map(|s| base64::engine::general_purpose::STANDARD.encode(s))
    }

    /// The step these header values make; `None` for anything malformed. An offer or answer
    /// needs its session description; the others carry none.
    pub fn from_headers(call: Option<&str>, sdp: Option<&str>) -> Option<CallSignal> {
        use base64::Engine as _;
        let (step, rest) = call?.trim().split_once(';')?;
        let step = CallStep::from_name(step.trim())?;
        let room = token(rest.trim().strip_prefix("room=")?)?;
        let sdp = match step {
            CallStep::Offer | CallStep::Answer => {
                let b64: String = sdp?.chars().filter(|c| !c.is_whitespace()).collect();
                if b64.len() > MAX_SDP * 4 / 3 + 4 {
                    return None;
                }
                let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
                let text = String::from_utf8(bytes).ok()?;
                Some(text).filter(|t| !t.is_empty() && t.len() <= MAX_SDP && t.starts_with("v=0"))
            }
            _ => None,
        };
        if matches!(step, CallStep::Offer | CallStep::Answer) && sdp.is_none() {
            return None;
        }
        Some(CallSignal { room, step, sdp })
    }
}

/// A step of the handshake that makes an invitation link into a verified chat (Delta Chat's
/// Secure-Join, setup-contact): `vc-request`, `vc-auth-required`, `vc-request-with-auth`
/// and `vc-contact-confirm`, with the tokens from the link and the joiner's fingerprint.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    pub step: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invitenumber: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    /// Upper-case hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// Most characters a handshake token may have; Delta Chat makes them about 24.
pub const MAX_TOKEN: usize = 100;

impl Handshake {
    /// The handshake step these `Secure-Join` header values make; `None` without a step. A
    /// field that is not a plain token is left out.
    pub fn from_headers(step: Option<&str>, invitenumber: Option<&str>, auth: Option<&str>, fingerprint: Option<&str>) -> Option<Handshake> {
        let step = step.map(str::trim).filter(|s| !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_lowercase() || c == '-'))?;
        Some(Handshake {
            step: step.to_string(),
            invitenumber: invitenumber.and_then(token),
            auth: auth.and_then(token),
            fingerprint: fingerprint.and_then(fingerprint_of),
        })
    }
}

/// `value` when it is a plain token: letters, digits, `-` and `_`.
pub fn token(value: &str) -> Option<String> {
    let v = value.trim();
    (!v.is_empty() && v.len() <= MAX_TOKEN && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))).then(|| v.to_string())
}

/// An OpenPGP fingerprint in upper-case hex without spaces, from one written with or without
/// them; `None` for anything else.
pub fn fingerprint_of(value: &str) -> Option<String> {
    let hex: String = value.chars().filter(|c| !c.is_whitespace() && *c != ':').collect();
    (matches!(hex.len(), 40 | 64) && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| hex.to_ascii_uppercase())
}

impl ChatTag {
    /// The tag a message with these header values carries; `None` without `Chat-Version`. A
    /// group id is only taken when it is a plain token, so it can be used as a key.
    pub fn from_headers(version: Option<&str>, group_id: Option<&str>, group_name: Option<&str>) -> Option<ChatTag> {
        version.filter(|v| !v.trim().is_empty())?;
        let group_id = group_id
            .map(str::trim)
            .filter(|id| !id.is_empty() && id.len() <= MAX_GROUP_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
            .map(str::to_string);
        let group_name = group_id
            .as_ref()
            .and(group_name)
            .map(|n| clean_text(n.trim()).chars().take(MAX_GROUP_NAME).collect::<String>())
            .filter(|n| !n.is_empty());
        Some(ChatTag { group_id, group_name, ..ChatTag::default() })
    }

    /// A new group id: 16 characters from `random`, of the alphabet Delta Chat uses.
    pub fn new_group_id(random: [u8; 16]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        random.iter().map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_comes_from_chat_version_and_keeps_only_a_clean_group() {
        assert_eq!(ChatTag::from_headers(None, Some("abc"), Some("x")), None);
        assert_eq!(ChatTag::from_headers(Some(" "), None, None), None);
        assert_eq!(ChatTag::from_headers(Some("1.0"), None, Some("Naam zonder groep")), Some(ChatTag::default()));
        let group = ChatTag::from_headers(Some("1.0"), Some(" Ab-9_x "), Some(" Familie\u{7} Rees ")).unwrap();
        assert_eq!(group.group_id.as_deref(), Some("Ab-9_x"));
        assert_eq!(group.group_name.as_deref(), Some("Familie Rees"));
        assert_eq!(ChatTag::from_headers(Some("1.0"), Some("a/b"), Some("x")), Some(ChatTag::default()), "not a token");
        assert_eq!(ChatTag::from_headers(Some("1.0"), Some(&"a".repeat(65)), None), Some(ChatTag::default()));
    }

    #[test]
    fn a_call_step_round_trips_through_its_headers_and_rejects_junk() {
        let offer = CallSignal { room: "Ab-9_x".into(), step: CallStep::Offer, sdp: Some("v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\n".into()) };
        assert_eq!(offer.header(), "offer; room=Ab-9_x");
        let sdp = offer.sdp_header().unwrap();
        assert_eq!(CallSignal::from_headers(Some(&offer.header()), Some(&format!("{} \r\n {}", &sdp[..10], &sdp[10..]))), Some(offer));
        let join = CallSignal::from_headers(Some(" join ;room=R1"), Some("djA=")).unwrap();
        assert_eq!((join.step, join.sdp), (CallStep::Join, None), "a join carries no description");
        assert_eq!(CallSignal::from_headers(Some("offer; room=R1"), None), None, "an offer needs its description");
        assert_eq!(CallSignal::from_headers(Some("offer; room=R1"), Some("aGFsbG8=")), None, "not a session description");
        assert_eq!(CallSignal::from_headers(Some("dial; room=R1"), None), None);
        assert_eq!(CallSignal::from_headers(Some("ring; room=a/b"), None), None);
    }

    #[test]
    fn a_new_group_id_is_a_token() {
        let id = ChatTag::new_group_id([255; 16]);
        assert_eq!(id.len(), 16);
        assert!(ChatTag::from_headers(Some("1.0"), Some(&id), None).unwrap().group_id.is_some());
    }

    #[test]
    fn a_handshake_keeps_only_plain_tokens_and_a_real_fingerprint() {
        assert_eq!(Handshake::from_headers(None, Some("x"), None, None), None);
        let h = Handshake::from_headers(Some(" vc-request "), Some("Ab_9-x"), Some("a b"), Some("1234 5678 9abc def0 1234 5678 9ABC DEF0 1234 5678"))
            .unwrap();
        assert_eq!(h.step, "vc-request");
        assert_eq!(h.invitenumber.as_deref(), Some("Ab_9-x"));
        assert_eq!(h.auth, None, "not a token");
        assert_eq!(h.fingerprint.as_deref(), Some("123456789ABCDEF0123456789ABCDEF012345678"));
        assert_eq!(Handshake::from_headers(Some("VC;x"), None, None, None), None);
        assert_eq!(fingerprint_of("abc"), None);
    }
}
