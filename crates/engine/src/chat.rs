//! Chat over e-mail, as Delta Chat does it: a chat message is an end-to-end encrypted message
//! with a `Chat-Version` header ([`ChatTag`]), sent and fetched like any other mail, so a chat
//! needs no server of its own and the other side can use Delta Chat on any device.
//!
//! The chats are made from the listed messages that carry the tag: one per group, else one per
//! person the account writes with. A copy in Verzonden items and the one the server delivered
//! back count once. A chat message only goes out encrypted, to people whose key Merel has; for
//! anyone else Merel says so instead of sending in the clear.

use std::collections::{BTreeMap, HashSet};

use merel_model::{Address, ChatTag, Draft, FolderKind, MessageHeader};
use merel_pgp::{KeyInfo, SecretKey};

use crate::{Engine, SAMPLE_ACCOUNT, Session};

/// The subject chat messages go out with; Delta Chat shows the text, not the subject.
pub const CHAT_SUBJECT: &str = "Chatbericht";
/// Most people in a group chat Merel starts.
pub const MAX_CHAT_PEOPLE: usize = 50;

/// A conversation of chat messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chat {
    /// `group:<account>:<group id>` or `person:<account>:<address>`.
    pub id: String,
    pub account_id: String,
    /// The group's name, else the other person's name or address.
    pub title: String,
    /// Everyone in the chat but the account itself.
    pub people: Vec<Address>,
    pub group: Option<ChatTag>,
    /// The messages, oldest first.
    pub messages: Vec<String>,
    /// When the last message came, in seconds since the Unix epoch.
    pub last: i64,
    pub unread: usize,
}

impl Chat {
    pub fn is_group(&self) -> bool {
        self.group.is_some()
    }
}

/// The id of the chat with `person` on `account_id`.
pub fn person_chat_id(account_id: &str, person: &str) -> String {
    format!("person:{account_id}:{}", person.trim().to_lowercase())
}

/// The id of the group chat `group_id` on `account_id`.
pub fn group_chat_id(account_id: &str, group_id: &str) -> String {
    format!("group:{account_id}:{group_id}")
}

impl Session {
    /// Every chat of every account, the one with the latest message first.
    pub fn chats(&self) -> Vec<Chat> {
        let mut chats: BTreeMap<String, Chat> = BTreeMap::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut messages: Vec<&MessageHeader> = self.messages.iter().filter(|m| m.chat.is_some()).collect();
        messages.sort_by_key(|m| m.date);
        for m in messages {
            let Some(folder) = self.folder(&m.folder_id) else { continue };
            if matches!(folder.kind, FolderKind::Junk | FolderKind::Deleted | FolderKind::Drafts) {
                continue;
            }
            let Some(account) = self.accounts.iter().find(|a| a.id == folder.account_id) else { continue };
            if let Some(mid) = &m.message_id
                && !seen.insert((account.id.clone(), mid.clone()))
            {
                continue;
            }
            let me = account.address.email().to_lowercase();
            let mine = m.from.email().eq_ignore_ascii_case(&me);
            let mut people: Vec<Address> = Vec::new();
            for a in std::iter::once(&m.from).chain(&m.to) {
                if !a.email().eq_ignore_ascii_case(&me) && !people.iter().any(|p| p.email().eq_ignore_ascii_case(a.email())) {
                    people.push(a.clone());
                }
            }
            let tag = m.chat.clone().unwrap_or_default();
            let (id, group) = match &tag.group_id {
                Some(g) => (group_chat_id(&account.id, g), Some(tag.clone())),
                None => {
                    // Between two people: the other one, whoever wrote.
                    let Some(other) = (if mine { m.to.iter().find(|a| !a.email().eq_ignore_ascii_case(&me)) } else { Some(&m.from) }) else {
                        continue;
                    };
                    people = vec![other.clone()];
                    (person_chat_id(&account.id, other.email()), None)
                }
            };
            let chat = chats.entry(id.clone()).or_insert_with(|| Chat {
                id,
                account_id: account.id.clone(),
                title: String::new(),
                people: Vec::new(),
                group: group.clone(),
                messages: Vec::new(),
                last: 0,
                unread: 0,
            });
            for p in people {
                if !chat.people.iter().any(|q| q.email().eq_ignore_ascii_case(p.email())) {
                    chat.people.push(p);
                }
            }
            // The latest name a group was given wins.
            if let Some(g) = group.filter(|g| g.group_name.is_some()) {
                chat.group = Some(g);
            }
            chat.messages.push(m.id.clone());
            chat.last = chat.last.max(m.date);
            if !m.flags.seen && !mine {
                chat.unread += 1;
            }
        }
        let mut out: Vec<Chat> = chats
            .into_values()
            .map(|mut c| {
                c.title = match c.group.as_ref().and_then(|g| g.group_name.clone()) {
                    Some(name) => name,
                    None if c.is_group() => c.people.iter().map(Address::display).collect::<Vec<_>>().join(", "),
                    None => c.people.first().map(|p| p.display().to_string()).unwrap_or_default(),
                };
                c
            })
            .collect();
        out.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.title.cmp(&b.title)));
        out
    }

    /// The chat with id `id`.
    pub fn chat(&self, id: &str) -> Option<Chat> {
        self.chats().into_iter().find(|c| c.id == id)
    }

    /// A chat message with `text` for `chat`, as an answer to its last message.
    pub fn chat_reply(&self, chat: &Chat, text: &str) -> Draft {
        let last = chat.messages.last().and_then(|id| self.messages.iter().find(|m| &m.id == id)).and_then(|m| m.message_id.clone());
        chat_draft(chat.people.clone(), chat.group.clone().unwrap_or_default(), text, last)
    }
}

/// A new chat with `people`: between two people for one, a group for more, named `name` when
/// given. A group gets a fresh id.
pub fn new_chat(people: &[Address], name: &str, text: &str) -> Result<Draft, String> {
    let mut unique: Vec<Address> = Vec::new();
    for p in people {
        if !unique.iter().any(|q| q.email().eq_ignore_ascii_case(p.email())) {
            unique.push(p.clone());
        }
    }
    if unique.is_empty() {
        return Err("Kies ten minste één persoon om mee te chatten.".into());
    }
    if unique.len() > MAX_CHAT_PEOPLE {
        return Err(format!("Een chat kan met hoogstens {MAX_CHAT_PEOPLE} personen."));
    }
    let name = merel_model::clean_text(name.trim()).chars().take(merel_model::MAX_CHAT_GROUP_NAME).collect::<String>();
    let tag = if unique.len() > 1 || !name.is_empty() {
        let mut random = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
        let group_name = if name.is_empty() { unique.iter().map(Address::display).collect::<Vec<_>>().join(", ") } else { name };
        ChatTag { group_id: Some(ChatTag::new_group_id(random)), group_name: Some(group_name) }
    } else {
        ChatTag::default()
    };
    Ok(chat_draft(unique, tag, text, None))
}

fn chat_draft(people: Vec<Address>, tag: ChatTag, text: &str, in_reply_to: Option<String>) -> Draft {
    Draft {
        to: people,
        subject: CHAT_SUBJECT.into(),
        body: text.to_string(),
        in_reply_to,
        sign: Some(true),
        encrypt: true,
        chat: Some(tag),
        ..Draft::default()
    }
}

impl Engine {
    /// Whether `account_id` can chat: it has an OpenPGP key.
    pub fn chat_ready(&self, account_id: &str) -> bool {
        self.pgp_key(account_id).is_some()
    }

    /// Turns chat on for `account_id`: it keeps the OpenPGP key it has, or gets a new one made
    /// for its address, which from then on goes along with its mail (Autocrypt) so others can
    /// write to it encrypted.
    pub fn enable_chat(&mut self, account_id: &str) -> Result<KeyInfo, String> {
        if let Some(info) = self.pgp_key(account_id) {
            return Ok(info);
        }
        let account = self
            .session
            .accounts
            .iter()
            .find(|a| a.id == account_id && a.id != SAMPLE_ACCOUNT)
            .ok_or_else(|| "Dit account bestaat niet meer.".to_string())?;
        let key = SecretKey::generate(&account.name, account.address.email()).map_err(|e| e.to_string())?;
        self.keep_pgp_key(account_id, key)
    }

    /// Sends the chat message `draft` from `account_id`. It only goes out encrypted: without
    /// the account's own key or a key for each person, it does not go out and says why.
    pub fn send_chat(&mut self, account_id: &str, draft: Draft) -> Result<u64, String> {
        if draft.chat.is_none() || !draft.encrypt {
            return Err("Een chatbericht gaat alleen versleuteld de deur uit.".into());
        }
        if draft.body.trim().is_empty() && draft.attachments.is_empty() {
            return Err("Typ eerst een bericht.".into());
        }
        if !self.chat_ready(account_id) {
            return Err("Zet eerst chat aan voor dit account.".into());
        }
        let missing = self.keys.openpgp.missing(draft.to.iter(), crate::drafts::now());
        if !missing.is_empty() {
            return Err(missing_chat_text(&missing));
        }
        Ok(self.send(account_id, draft))
    }
}

/// Says with whom no encrypted chat is possible yet.
pub(crate) fn missing_chat_text(missing: &[String]) -> String {
    match missing {
        [one] => format!("Met {one} kun je nog niet versleuteld chatten. Stuur eerst een uitnodiging."),
        many => format!("Met {} kun je nog niet versleuteld chatten. Stuur eerst een uitnodiging.", many.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use merel_model::{Account, Auth, Flags, Folder, Protocol, Server, Tls};
    use merel_secrets::Secret;

    use super::*;

    const JIJ_PUBLIC: &[u8] = include_bytes!("../../pgp/testdata/jij-public.asc");
    const SIGNED: &[u8] = include_bytes!("../../pgp/testdata/signed-gpg.eml");

    fn addr(email: &str) -> Address {
        Address::new(None, email).unwrap()
    }

    fn account() -> Account {
        Account {
            id: "a1".into(),
            name: "Alain Rees".into(),
            address: Address::new(Some("Alain Rees"), "alain@voorbeeld.nl").unwrap(),
            username: "alain@voorbeeld.nl".into(),
            protocol: Protocol::ImapSmtp,
            auth: Auth::Password,
            incoming: Some(Server { host: "127.0.0.1".into(), port: 1, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        }
    }

    fn folder(id: &str, kind: FolderKind) -> Folder {
        Folder { id: id.into(), account_id: "a1".into(), name: id.into(), path: id.into(), kind, unread: 0, parent: None }
    }

    fn message(id: &str, folder: &str, from: &str, to: &[&str], date: i64, chat: Option<ChatTag>) -> MessageHeader {
        MessageHeader {
            id: id.into(),
            folder_id: folder.into(),
            remote_id: id.into(),
            from: addr(from),
            to: to.iter().map(|t| addr(t)).collect(),
            subject: CHAT_SUBJECT.into(),
            date,
            preview: String::new(),
            flags: Flags { seen: false, flagged: false, answered: false },
            has_attachments: false,
            message_id: Some(format!("<{id}@voorbeeld.nl>")),
            categories: Vec::new(),
            importance: merel_model::Importance::Normal,
            chat,
        }
    }

    fn session() -> Session {
        let mut s = Session { accounts: vec![account()], ..Session::default() };
        s.folders = vec![folder("in", FolderKind::Inbox), folder("sent", FolderKind::Sent), folder("junk", FolderKind::Junk)];
        let chat = Some(ChatTag::default());
        let group = |name: Option<&str>| Some(ChatTag { group_id: Some("G1".into()), group_name: name.map(str::to_string) });
        s.messages = vec![
            message("m1", "in", "sanne@example.org", &["alain@voorbeeld.nl"], 100, chat.clone()),
            message("m2", "sent", "alain@voorbeeld.nl", &["sanne@example.org"], 200, chat.clone()),
            // The server's copy of m2 in the inbox counts once.
            MessageHeader {
                id: "m2b".into(),
                folder_id: "in".into(),
                ..message("m2", "in", "alain@voorbeeld.nl", &["sanne@example.org"], 200, chat.clone())
            },
            message("m3", "in", "ravi@example.org", &["alain@voorbeeld.nl", "sanne@example.org"], 300, group(Some("Ontwerpteam"))),
            message("m4", "in", "sanne@example.org", &["alain@voorbeeld.nl", "ravi@example.org"], 400, group(None)),
            message("m5", "in", "nieuws@example.org", &["alain@voorbeeld.nl"], 500, None),
            message("m6", "junk", "spam@example.org", &["alain@voorbeeld.nl"], 600, chat),
        ];
        s
    }

    #[test]
    fn chat_messages_make_chats_per_person_and_per_group() {
        let chats = session().chats();
        assert_eq!(chats.len(), 2, "{chats:#?}");
        let group = &chats[0];
        assert_eq!(group.id, "group:a1:G1");
        assert_eq!(group.title, "Ontwerpteam", "a later message without a name keeps the name");
        assert_eq!(group.messages, vec!["m3", "m4"]);
        let people: Vec<&str> = group.people.iter().map(|p| p.email()).collect();
        assert_eq!(people, vec!["ravi@example.org", "sanne@example.org"]);
        assert_eq!(group.unread, 2);

        let sanne = &chats[1];
        assert_eq!(sanne.id, person_chat_id("a1", "Sanne@example.org"));
        assert_eq!(sanne.title, "sanne@example.org");
        assert_eq!(sanne.messages, vec!["m1", "m2"], "the copy counts once; plain mail and junk are no chat");
        assert_eq!(sanne.unread, 1, "an own message is never unread");
        assert_eq!(sanne.last, 200);
    }

    #[test]
    fn an_answer_and_a_new_chat_are_encrypted_chat_drafts() {
        let s = session();
        let sanne = s.chat(&person_chat_id("a1", "sanne@example.org")).unwrap();
        let reply = s.chat_reply(&sanne, "Tot straks");
        assert_eq!(reply.to, vec![addr("sanne@example.org")]);
        assert_eq!(reply.chat, Some(ChatTag::default()));
        assert!(reply.encrypt && reply.sign == Some(true));
        assert_eq!(reply.in_reply_to.as_deref(), Some("<m2@voorbeeld.nl>"));
        let group = s.chat("group:a1:G1").unwrap();
        assert_eq!(s.chat_reply(&group, "x").chat.and_then(|t| t.group_id).as_deref(), Some("G1"));

        assert!(new_chat(&[], "", "x").is_err());
        let one = new_chat(&[addr("ravi@example.org"), addr("Ravi@example.org")], "", "Hoi").unwrap();
        assert_eq!((one.to.len(), one.chat.clone()), (1, Some(ChatTag::default())));
        let many = new_chat(&[addr("ravi@example.org"), addr("sanne@example.org")], "", "Hoi").unwrap();
        let tag = many.chat.unwrap();
        assert_eq!(tag.group_name.as_deref(), Some("ravi@example.org, sanne@example.org"));
        assert_eq!(tag.group_id.map(|g| g.len()), Some(16));
        let named = new_chat(&[addr("ravi@example.org")], "Kwartaal", "Hoi").unwrap();
        assert_eq!(named.chat.and_then(|t| t.group_name).as_deref(), Some("Kwartaal"), "a name makes a group");
    }

    #[test]
    fn a_chat_message_from_the_server_is_recognised() {
        let raw = b"From: Sanne <sanne@example.org>\r\nTo: alain@voorbeeld.nl\r\nSubject: [...]\r\nChat-Version: 1.0\r\nChat-Group-ID: Ab-9_x\r\nChat-Group-Name: =?utf-8?q?Caf=C3=A9_Rees?=\r\n\r\n";
        let header = merel_mime::parse_header(raw);
        assert_eq!(header.chat, Some(ChatTag { group_id: Some("Ab-9_x".into()), group_name: Some("Café Rees".into()) }));
        assert_eq!(merel_mime::parse_header(b"From: a@b.nl\r\nSubject: x\r\n\r\n").chat, None);
    }

    #[test]
    fn chat_only_goes_out_encrypted_to_people_with_a_key() {
        let mut e = Engine::sample();
        e.add_account(account(), Secret::new("x")).unwrap();
        let draft = new_chat(&[addr("jij@example.org")], "", "Hoi").unwrap();
        assert_eq!(e.send_chat("a1", draft.clone()).unwrap_err(), "Zet eerst chat aan voor dit account.");
        assert!(!e.chat_ready("a1"));
        let info = e.enable_chat("a1").unwrap();
        assert_eq!(info.emails, vec!["alain@voorbeeld.nl".to_string()]);
        assert_eq!(info.name, "Alain Rees");
        assert!(e.chat_ready("a1"));
        assert_eq!(e.enable_chat("a1").unwrap(), info, "the key stays");

        assert_eq!(e.send_chat("a1", Draft { encrypt: false, ..draft.clone() }).unwrap_err(), "Een chatbericht gaat alleen versleuteld de deur uit.");
        assert_eq!(
            e.send_chat("a1", draft.clone()).unwrap_err(),
            "Met jij@example.org kun je nog niet versleuteld chatten. Stuur eerst een uitnodiging."
        );
        let key =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, merel_pgp::PublicKey::from_bytes(JIJ_PUBLIC).unwrap().to_bytes());
        let mut raw = format!("Autocrypt: addr=jij@example.org; keydata={key}\r\n").into_bytes();
        raw.extend_from_slice(SIGNED);
        e.learn_autocrypt("x", Some(&addr("jij@example.org")), &raw);
        assert!(e.send_chat("a1", draft.clone()).is_ok());

        // What goes on the wire: encrypted, with the chat header outside for the other side.
        let sent =
            crate::certificates::outgoing(&e.keys, &account(), &draft, merel_smtp::build(&account().address, &draft).unwrap().formatted()).unwrap();
        assert!(merel_pgp::is_encrypted(&sent) && !sent.windows(4).any(|w| w == b"Hoi\r"));
        assert_eq!(merel_mime::parse_header(&sent).chat, Some(ChatTag::default()));
        let plain_mail = Draft { encrypt: false, chat: None, ..draft.clone() };
        assert!(crate::certificates::outgoing(&e.keys, &account(), &Draft { chat: Some(ChatTag::default()), ..plain_mail }, Vec::new()).is_err());
    }
}
