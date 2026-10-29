//! Chat over e-mail, as Delta Chat does it: a chat message is an end-to-end encrypted message
//! with a `Chat-Version` header ([`ChatTag`]), sent and fetched like any other mail, so a chat
//! needs no server of its own and the other side can use Delta Chat on any device.
//!
//! The chats are made from the listed messages that carry the tag: one per group, else one per
//! person the account writes with. A copy in Verzonden items and the one the server delivered
//! back count once. A chat message only goes out encrypted, to people whose key Merel has; for
//! anyone else Merel says so instead of sending in the clear.

use std::collections::{BTreeMap, HashSet};

use merel_model::{Address, ChatTag, Draft, FolderKind, MessageHeader, Sealed};
use merel_pgp::{KeyInfo, SecretKey};

use crate::{Engine, SAMPLE_ACCOUNT, Session};

/// The subject chat messages go out with; Delta Chat shows the text, not the subject.
pub const CHAT_SUBJECT: &str = "Chatbericht";
/// Most people in a group chat Merel starts.
pub const MAX_CHAT_PEOPLE: usize = 50;
/// How many of a chat's latest messages are fetched when it opens.
pub const CHAT_LOAD: usize = 100;

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
        // The steps of an invitation's handshake are no messages to show, nor those of a
        // meeting but its ring.
        let mut messages: Vec<&MessageHeader> = self
            .messages
            .iter()
            .filter(|m| {
                m.chat.as_ref().is_some_and(|c| c.handshake.is_none() && c.call.as_ref().is_none_or(|k| k.step == merel_model::CallStep::Ring))
            })
            .collect();
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

    /// Opens the chat `id`: asks for the text of its latest messages and marks them read.
    pub fn open_chat(&mut self, id: &str) {
        let Some(chat) = self.chat(id) else { return };
        for message in chat.messages.iter().rev().take(CHAT_LOAD) {
            self.request_body(message);
            let _ = crate::commands::set_seen_one(self, message, true);
        }
    }

    /// The text of the chat message `id`, once fetched: without the status line Delta Chat
    /// puts under `-- `, and without trailing blank lines.
    pub fn chat_text(&self, id: &str) -> Option<String> {
        let text = self.bodies.get(id)?.text.replace("\r\n", "\n");
        let text = match text.find("\n-- \n") {
            Some(cut) => &text[..cut],
            None => text.strip_prefix("-- \n").map(|_| "").unwrap_or(&text),
        };
        Some(text.trim_end().to_string())
    }

    /// Whether the chat message `id` came from the account it is listed under.
    pub fn chat_is_mine(&self, id: &str) -> bool {
        let Some(m) = self.message(id) else { return false };
        let account = self.folder(&m.folder_id).and_then(|f| self.accounts.iter().find(|a| a.id == f.account_id));
        account.is_some_and(|a| a.address.email().eq_ignore_ascii_case(m.from.email()))
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
        return Err(merel_i18n::tr("chat-pick-someone"));
    }
    if unique.len() > MAX_CHAT_PEOPLE {
        return Err(merel_i18n::tr_args("chat-too-many", &[("max", MAX_CHAT_PEOPLE.into())]));
    }
    let name = merel_model::clean_text(name.trim()).chars().take(merel_model::MAX_CHAT_GROUP_NAME).collect::<String>();
    let tag = if unique.len() > 1 || !name.is_empty() {
        let mut random = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
        let group_name = if name.is_empty() { unique.iter().map(Address::display).collect::<Vec<_>>().join(", ") } else { name };
        ChatTag { group_id: Some(ChatTag::new_group_id(random)), group_name: Some(group_name), ..ChatTag::default() }
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

/// Whether `account` lists the chat headers: an IMAP account, or Microsoft 365, whose Graph
/// client reads them apart. Exchange and Gmail list mail through interfaces that do not hand
/// them over yet.
pub fn can_chat(account: &merel_model::Account) -> bool {
    matches!(account.protocol, merel_model::Protocol::ImapSmtp | merel_model::Protocol::Microsoft365) && account.id != SAMPLE_ACCOUNT
}

/// Most encrypted messages opened after one listing to read the header inside.
const OPEN_SEALED: usize = 30;

impl Engine {
    /// Opens the newest encrypted messages in `folder_id` whose header inside has not been
    /// read yet, for an account with chat on: Delta Chat names a chat only inside the
    /// encryption, so this is how its chats show up.
    pub(crate) fn open_sealed_in(&mut self, folder_id: &str) {
        let Some(folder) = self.session.folder(folder_id) else { return };
        if matches!(folder.kind, FolderKind::Junk | FolderKind::Deleted | FolderKind::Drafts) || !self.chat_ready(&folder.account_id) {
            return;
        }
        // The first step of an invitation comes unencrypted; it is opened to be answered.
        let recent = crate::drafts::now() - 7 * 24 * 3600;
        let handshake = |m: &MessageHeader| m.chat.as_ref().is_some_and(|c| c.handshake.is_some()) && m.date > recent;
        let mut sealed: Vec<(i64, String)> = self
            .session
            .messages
            .iter()
            .filter(|m| m.folder_id == folder_id && (m.sealed == Sealed::Closed || handshake(m)))
            .map(|m| (m.date, m.id.clone()))
            .collect();
        sealed.sort_by_key(|m| std::cmp::Reverse(m.0));
        for (_, id) in sealed.into_iter().take(OPEN_SEALED) {
            self.session.request_body(&id);
        }
    }

    /// Lists the Verzonden items of every connected account with chat on, so chat messages
    /// sent from elsewhere (Delta Chat on a phone) join the chats too.
    pub(crate) fn refresh_chat_folders(&mut self) {
        let ids: Vec<String> = self
            .session
            .accounts
            .iter()
            .filter(|a| self.session.status.get(&a.id) == Some(&crate::AccountStatus::Online) && self.chat_ready(&a.id))
            .filter_map(|a| self.session.folder_of_kind(&a.id, FolderKind::Sent))
            .map(|f| f.id.clone())
            .collect();
        for id in ids {
            self.session.request_headers(&id);
        }
    }

    /// Opens the chat `id`; see [`Session::open_chat`].
    pub fn open_chat(&mut self, id: &str) {
        self.session.open_chat(id);
        self.dispatch();
    }

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
        let account =
            self.session.accounts.iter().find(|a| a.id == account_id && a.id != SAMPLE_ACCOUNT).ok_or_else(|| merel_i18n::tr("account-gone"))?;
        let key = SecretKey::generate(&account.name, account.address.email()).map_err(|e| e.to_string())?;
        self.keep_pgp_key(account_id, key)
    }

    /// Sends the chat message `draft` from `account_id`. It only goes out encrypted: without
    /// the account's own key or a key for each person, it does not go out and says why.
    pub fn send_chat(&mut self, account_id: &str, draft: Draft) -> Result<u64, String> {
        if self.is_signal_account(account_id) {
            let id = self.send_signal(account_id, draft)?;
            self.dispatch();
            return Ok(id);
        }
        if draft.chat.is_none() || !draft.encrypt {
            return Err(merel_i18n::tr("chat-encrypted-only"));
        }
        if draft.body.trim().is_empty() && draft.attachments.is_empty() {
            return Err(merel_i18n::tr("chat-type-first"));
        }
        if !self.chat_ready(account_id) {
            return Err(merel_i18n::tr("chat-turn-on-first"));
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
        [one] => merel_i18n::tr_args("chat-not-yet", &[("who", one.as_str().into())]),
        many => merel_i18n::tr_args("chat-not-yet", &[("who", many.join(", ").into())]),
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
            sealed: Default::default(),
            size: 0,
        }
    }

    fn session() -> Session {
        let mut s = Session { accounts: vec![account()], ..Session::default() };
        s.folders = vec![folder("in", FolderKind::Inbox), folder("sent", FolderKind::Sent), folder("junk", FolderKind::Junk)];
        let chat = Some(ChatTag::default());
        let group = |name: Option<&str>| Some(ChatTag { group_id: Some("G1".into()), group_name: name.map(str::to_string), ..ChatTag::default() });
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
    fn opening_a_chat_marks_it_read_and_its_text_drops_the_status_line() {
        let mut s = session();
        let id = person_chat_id("a1", "sanne@example.org");
        s.open_chat(&id);
        assert_eq!(s.chat(&id).unwrap().unread, 0);
        assert!(s.loading.contains("m1") && s.loading.contains("m2"), "the texts are fetched");
        s.bodies.insert("m1".into(), merel_mime::Body { text: "Tot straks\r\n\r\n-- \r\nVerstuurd met Delta Chat\r\n".into(), ..Default::default() });
        assert_eq!(s.chat_text("m1").as_deref(), Some("Tot straks"));
        assert_eq!(s.chat_text("m2"), None, "not fetched yet");
        assert!(s.chat_is_mine("m2") && !s.chat_is_mine("m1"));
        s.selected_folder = Some("in".into());
        assert!(s.visible_messages().iter().all(|m| m.chat.is_none()), "chat stays out of the mail list");
        assert!(s.visible_messages().iter().any(|m| m.id == "m5"));
    }

    #[test]
    fn a_chat_message_from_the_server_is_recognised() {
        let raw = b"From: Sanne <sanne@example.org>\r\nTo: alain@voorbeeld.nl\r\nSubject: [...]\r\nChat-Version: 1.0\r\nChat-Group-ID: Ab-9_x\r\nChat-Group-Name: =?utf-8?q?Caf=C3=A9_Rees?=\r\n\r\n";
        let header = merel_mime::parse_header(raw);
        assert_eq!(header.chat, Some(ChatTag { group_id: Some("Ab-9_x".into()), group_name: Some("Café Rees".into()), ..ChatTag::default() }));
        assert_eq!(merel_mime::parse_header(b"From: a@b.nl\r\nSubject: x\r\n\r\n").chat, None);
    }

    #[test]
    fn chat_only_goes_out_encrypted_to_people_with_a_key() {
        let mut e = Engine::sample();
        e.add_account(account(), Secret::new("x")).unwrap();
        let draft = new_chat(&[addr("jij@example.org")], "", "Hoi").unwrap();
        assert_eq!(e.send_chat("a1", draft.clone()).unwrap_err(), merel_i18n::tr("chat-turn-on-first"));
        assert!(!e.chat_ready("a1"));
        let info = e.enable_chat("a1").unwrap();
        assert_eq!(info.emails, vec!["alain@voorbeeld.nl".to_string()]);
        assert_eq!(info.name, "Alain Rees");
        assert!(e.chat_ready("a1"));
        assert_eq!(e.enable_chat("a1").unwrap(), info, "the key stays");

        assert_eq!(e.send_chat("a1", Draft { encrypt: false, ..draft.clone() }).unwrap_err(), merel_i18n::tr("chat-encrypted-only"));
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

    /// A message as Delta Chat sends it: everything inside the encryption, outside only the
    /// address, `[...]` as the subject and a date days off.
    fn from_delta_chat(sender: &SecretKey, to: &merel_pgp::PublicKey, inner: &str) -> Vec<u8> {
        let raw = merel_pgp::with_header(inner.as_bytes(), &sender.autocrypt_header("jij@example.org"));
        let sealed = merel_pgp::encrypt_message(sender, &raw, std::slice::from_ref(to), true, crate::drafts::now()).unwrap();
        let text = String::from_utf8(sealed).unwrap();
        let (outer, rest) = text.split_once("MIME-Version: 1.0\r\n").unwrap();
        let mut kept = String::new();
        let mut skipping = false;
        for line in outer.split_inclusive("\r\n") {
            if !line.starts_with([' ', '\t']) {
                let name = line.split(':').next().unwrap_or_default().to_ascii_lowercase();
                skipping = name.starts_with("chat-") || name == "autocrypt" || name == "subject" || name == "date";
            }
            if !skipping {
                kept.push_str(line);
            }
        }
        format!("{kept}Subject: [...]\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\nMIME-Version: 1.0\r\n{rest}").into_bytes()
    }

    fn listed(raw: &[u8]) -> merel_mailbox::RemoteHeader {
        let header = merel_mime::parse_header(raw);
        merel_mailbox::RemoteHeader {
            id: "1".into(),
            seen: false,
            flagged: false,
            answered: false,
            size: raw.len() as u32,
            has_attachments: true,
            date: header.date.unwrap_or(0),
            header,
            categories: Vec::new(),
        }
    }

    #[test]
    fn a_delta_chat_message_becomes_a_chat_once_its_header_inside_is_read() {
        let mut e = Engine::sample();
        e.add_account(account(), Secret::new("x")).unwrap();
        e.enable_chat("a1").unwrap();
        e.session.folders.push(Folder { account_id: "a1".into(), ..folder("a1/INBOX", FolderKind::Inbox) });
        let me = e.keys.openpgp.own_key("a1").unwrap().public();
        let jij = SecretKey::import(include_bytes!("../../pgp/testdata/jij-secret.asc"), "geheim").unwrap();
        let ravi = SecretKey::generate("Ravi", "ravi@example.org").unwrap().public();
        let raw = from_delta_chat(
            &jij,
            &me,
            &format!(
                "From: Jij <jij@example.org>\r\nTo: alain@voorbeeld.nl, ravi@example.org\r\nSubject: Fietsclub\r\n\
                 Date: Wed, 21 Oct 2026 20:15:00 +0000\r\nMessage-ID: <dc1@example.org>\r\nChat-Version: 1.0\r\nChat-Group-ID: G7\r\n\
                 Chat-Group-Name: Fietsclub\r\n{}Content-Type: text/plain; charset=utf-8\r\n\r\nZondag om tien uur?\r\n\r\n-- \r\nSent with Delta Chat\r\n",
                ravi.gossip_header("ravi@example.org")
            ),
        );
        let outside = String::from_utf8_lossy(&raw);
        assert!(!outside.contains("Chat-") && !outside.contains("Fietsclub") && !outside.contains("Autocrypt"), "{outside}");

        e.session.apply(crate::Event::Headers { folder_id: "a1/INBOX".into(), headers: vec![listed(&raw)], offset: 0, limit: 50 });
        let id = "a1/INBOX#1";
        assert_eq!(e.session.message(id).map(|m| (m.sealed, m.chat.is_some(), m.subject.as_str())), Some((Sealed::Closed, false, "[...]")));
        assert!(e.session.chats().is_empty(), "outside it is no chat yet");
        e.open_sealed_in("a1/INBOX");
        assert!(e.session.loading.contains(id), "an account with chat on opens it");

        // What the worker hands back once the message is fetched.
        let (body, header, carried) = crate::certificates::read(&e.keys, "a1", &raw);
        assert!(matches!(&body.pgp_signature, Some(merel_pgp::Verdict::Valid { signer }) if signer.covers("jij@example.org")));
        e.learn_carried(id, header.from.as_ref(), &body, &carried.unwrap());
        e.session.apply(crate::Event::Body { message_id: id.into(), body: Box::new(body), header: Box::new(header), raw: Vec::new(), carried: None });
        assert!(e.pgp_key_of("jij@example.org").is_some(), "the sender's key from inside");
        assert!(e.pgp_key_of("ravi@example.org").is_some(), "the others' keys from a signed message");
        let m = e.session.message(id).unwrap();
        assert_eq!((m.sealed, m.subject.as_str(), m.from.display()), (Sealed::Opened, "Fietsclub", "Jij"));
        assert_eq!(m.date, 1_792_613_700, "the real date, from inside");
        let chats = e.session.chats();
        assert_eq!(chats.len(), 1);
        assert_eq!((chats[0].id.as_str(), chats[0].title.as_str()), ("group:a1:G7", "Fietsclub"));
        assert_eq!(e.session.chat_text(id).as_deref(), Some("Zondag om tien uur?"));

        // The next listing still knows what was inside.
        e.session.apply(crate::Event::Headers { folder_id: "a1/INBOX".into(), headers: vec![listed(&raw)], offset: 0, limit: 50 });
        assert_eq!(e.session.chats().first().map(|c| c.title.clone()).as_deref(), Some("Fietsclub"));

        // An answer to the group tells everyone everyone's key, inside only.
        let reply = e.session.chat_reply(&chats[0], "Ik ben er");
        assert!(e.send_chat("a1", reply.clone()).is_ok());
        let sent =
            crate::certificates::outgoing(&e.keys, &account(), &reply, merel_smtp::build(&account().address, &reply).unwrap().formatted()).unwrap();
        assert!(!String::from_utf8_lossy(&sent).contains("Autocrypt-Gossip"));
        let opened = merel_pgp::open_message(&jij, &sent, std::slice::from_ref(&me)).unwrap();
        let inside = String::from_utf8_lossy(&opened.plain);
        assert!(inside.contains("Chat-Group-ID: G7") && inside.matches("Autocrypt-Gossip:").count() == 2, "{inside}");
    }

    #[test]
    fn a_key_brought_along_does_not_vouch_for_an_address_with_another_key() {
        let mut e = Engine::sample();
        e.add_account(account(), Secret::new("x")).unwrap();
        e.enable_chat("a1").unwrap();
        e.session.folders.push(Folder { account_id: "a1".into(), ..folder("a1/INBOX", FolderKind::Inbox) });
        let me = e.keys.openpgp.own_key("a1").unwrap().public();
        // Merel knows another key for jij@example.org already.
        let other = SecretKey::generate("Jij", "jij@example.org").unwrap();
        e.remember_key("jij@example.org".into(), other.public());
        let jij = SecretKey::import(include_bytes!("../../pgp/testdata/jij-secret.asc"), "geheim").unwrap();
        let raw = from_delta_chat(
            &jij,
            &me,
            "From: jij@example.org\r\nTo: alain@voorbeeld.nl\r\nSubject: Hoi\r\nChat-Version: 1.0\r\nContent-Type: text/plain\r\n\r\nHoi\r\n",
        );
        let (body, _, _) = crate::certificates::read(&e.keys, "a1", &raw);
        assert!(matches!(body.pgp_signature, Some(merel_pgp::Verdict::UnknownKey(_))), "{:?}", body.pgp_signature);
    }
}
