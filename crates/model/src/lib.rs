//! The mail model: accounts, folders, message headers and addresses.
//!
//! This crate does no I/O and knows nothing of the UI. Everything that crosses into it from the
//! network (an address, a subject, a folder name) is validated or bounded here, so the layers
//! above can trust what they hold.

mod address;
mod calendar;
mod categories;
mod chat;
mod contact;
mod groups;
mod junk;
mod quicksteps;
pub mod rich;
mod rules;
mod safety;
mod task;
mod templates;

pub use address::{Address, AddressError, parse_address_list};
pub use calendar::{
    Appointment, AppointmentError, Attendee, CALENDAR_COLORS, Calendar, Frequency, MAX_ATTENDEES, MAX_CALENDAR_NAME, MAX_INTERVAL, MAX_NOTES,
    MAX_OCCURRENCES, MAX_REMINDER, Meeting, Recurrence, Response, ShowAs, describe_recurrence,
};
pub use categories::{Category, CategoryColor, MAX_CATEGORY_NAME, MAX_ITEM_CATEGORIES, category_keyword, clean_categories};
pub use chat::{CHAT_VERSION, ChatTag, MAX_GROUP_ID, MAX_GROUP_NAME as MAX_CHAT_GROUP_NAME};
pub use contact::{Contact, ContactError};
pub use groups::{ContactGroup, ContactGroupError, MAX_GROUP_MEMBERS, MAX_GROUP_NAME, mailbox};
pub use junk::{JunkLevel, JunkLists, MAX_JUNK_ENTRY, entry_covers, junk_entry};
pub use quicksteps::{MAX_QUICK_STEP_NAME, QuickStep, QuickStepError};
pub use rules::{MAX_RULE_TEXT, Rule, RuleError, RuleField};
pub use safety::{RISKY_EXTENSIONS, Warning, attachment_warnings, authentication_warning, host_of, link_warnings, lookalike, sender_warnings};
pub use task::{Priority, Repeat, Task, TaskError};
pub use templates::{MAX_TEMPLATE_NAME, MAX_TEMPLATE_TEXT, Template, TemplateError};

use serde::{Deserialize, Serialize};

/// Longest subject or display name kept, in characters; a hostile server can send megabytes.
pub const MAX_TEXT: usize = 998;

/// How an account talks to its server. IMAP/SMTP comes first; the others follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    ImapSmtp,
    Gmail,
    Microsoft365,
    Exchange,
}

/// Transport security for a connection. There is deliberately no plain-text variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tls {
    /// TLS from the first byte (IMAPS 993, SMTPS 465).
    Implicit,
    /// STARTTLS, required: the connection is dropped if the server does not offer it.
    StartTls,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub tls: Tls,
}

/// How an account proves who it is. The secret itself (password or refresh token) lives in
/// the system keychain under the account id; this only says which kind it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Auth {
    Password,
    /// OAuth 2.0 with PKCE through the system browser; the keychain holds the refresh token.
    OAuth2 {
        client_id: String,
        auth_url: String,
        token_url: String,
        scope: String,
    },
}

/// Merel's own registration with Microsoft Entra: a multi-tenant public client (no secret,
/// loopback redirects only), so people sign in without registering anything themselves. Not a
/// secret: it travels in every sign-in link.
pub const MICROSOFT_CLIENT_ID: &str = "88e91d33-f14d-4f27-8820-68b8c486c5a8";

impl Protocol {
    /// The client ID this build signs in with when the person names none of their own. A build
    /// can bring its own registration through `MEREL_MICROSOFT_CLIENT_ID`.
    pub fn builtin_client_id(self) -> Option<&'static str> {
        match self {
            Protocol::Microsoft365 => {
                [option_env!("MEREL_MICROSOFT_CLIENT_ID"), Some(MICROSOFT_CLIENT_ID)].into_iter().flatten().map(str::trim).find(|id| !id.is_empty())
            }
            _ => None,
        }
    }

    /// The ready-made settings for a hosted service; `None` for plain IMAP/SMTP.
    pub fn preset(self) -> Option<Preset> {
        match self {
            Protocol::ImapSmtp | Protocol::Exchange => None,
            Protocol::Gmail => Some(Preset {
                incoming: Server { host: "imap.gmail.com".into(), port: 993, tls: Tls::Implicit },
                outgoing: Some(Server { host: "smtp.gmail.com".into(), port: 465, tls: Tls::Implicit }),
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token_url: "https://oauth2.googleapis.com/token".into(),
                // IMAP and SMTP, then the Calendar, People and Tasks APIs.
                scope: "https://mail.google.com/ https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/contacts https://www.googleapis.com/auth/tasks".into(),
            }),
            // Everything through Microsoft Graph: mail, sending, calendar, contacts and tasks
            // are one OAuth resource, and Exchange Online stops answering EWS in October 2026.
            Protocol::Microsoft365 => Some(Preset {
                incoming: Server { host: "graph.microsoft.com".into(), port: 443, tls: Tls::Implicit },
                outgoing: None,
                auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
                token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
                scope: "https://graph.microsoft.com/Mail.ReadWrite https://graph.microsoft.com/Mail.Send https://graph.microsoft.com/Calendars.ReadWrite https://graph.microsoft.com/Contacts.ReadWrite https://graph.microsoft.com/Tasks.ReadWrite offline_access".into(),
            }),
        }
    }
}

/// Servers and OAuth endpoints of a hosted mail service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub incoming: Server,
    /// `None` when the store sends itself.
    pub outgoing: Option<Server>,
    pub auth_url: String,
    pub token_url: String,
    pub scope: String,
}

/// An account. Secrets (passwords, OAuth tokens) are never stored here: they live in the
/// system keychain, looked up by `id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub address: Address,
    /// The login name, usually the address itself.
    pub username: String,
    pub protocol: Protocol,
    #[serde(default = "Auth::password")]
    pub auth: Auth,
    pub incoming: Option<Server>,
    pub outgoing: Option<Server>,
    /// A CalDAV/CardDAV server next to an IMAP account, for its calendar, contacts and tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dav: Option<Dav>,
    /// Text put under every message written from this account; empty for none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
}

/// How long a signature may be, in characters.
pub const MAX_SIGNATURE: usize = 2_000;

/// A signature as it is stored: line breaks and tabs kept, other control characters and
/// surrounding blank lines gone, bounded.
pub fn clean_signature(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(MAX_SIGNATURE)
        .collect::<String>()
        .trim_matches(['\n', ' ', '\t'])
        .to_string()
}

/// Where an IMAP account's calendars and address books live (CalDAV and CardDAV); the same
/// user name and password sign in there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dav {
    /// The server's `https://` address; discovery starts at its `/.well-known/caldav`.
    pub calendar: String,
    /// The CardDAV server, when it is another one (iCloud, Fastmail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contacts: Option<String>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DavError {
    #[error("a DAV address starts with https:// and names a server")]
    BadUrl,
}

impl Dav {
    /// Checks both addresses: `https://`, a host, no spaces or control characters, bounded.
    pub fn new(calendar: &str, contacts: &str) -> Result<Dav, DavError> {
        fn check(url: &str) -> Result<String, DavError> {
            let url = url.trim();
            let host = url.strip_prefix("https://").ok_or(DavError::BadUrl)?;
            let ok = host.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
                && url.chars().count() <= 512
                && !url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '<' || c == '>');
            if ok { Ok(url.to_string()) } else { Err(DavError::BadUrl) }
        }
        let calendar = check(calendar)?;
        let contacts = if contacts.trim().is_empty() { None } else { Some(check(contacts)?) };
        Ok(Dav { calendar, contacts })
    }
}

impl Auth {
    fn password() -> Auth {
        Auth::Password
    }
}

/// The folders Outlook treats specially; everything else is `Other`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderKind {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Deleted,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    /// What the list shows: the last path segment, or the Dutch name of a special folder.
    pub name: String,
    /// The mailbox name on the server, as given to SELECT.
    pub path: String,
    pub kind: FolderKind,
    pub unread: u32,
    /// The id of the folder this one sits in, as the folder pane nests it; `None` at the top.
    #[serde(default)]
    pub parent: Option<String>,
}

impl FolderKind {
    /// The name Outlook (Dutch) gives a special folder; `None` for ordinary ones.
    pub fn dutch_name(self) -> Option<&'static str> {
        Some(match self {
            FolderKind::Inbox => "Postvak IN",
            FolderKind::Drafts => "Concepten",
            FolderKind::Sent => "Verzonden items",
            FolderKind::Archive => "Archief",
            FolderKind::Junk => "Ongewenste e-mail",
            FolderKind::Deleted => "Verwijderde items",
            FolderKind::Other => return None,
        })
    }

    /// Order in the folder pane: Inbox first, then the other special folders, then the rest.
    pub fn rank(self) -> u8 {
        match self {
            FolderKind::Inbox => 0,
            FolderKind::Drafts => 1,
            FolderKind::Sent => 2,
            FolderKind::Archive => 3,
            FolderKind::Junk => 4,
            FolderKind::Deleted => 5,
            FolderKind::Other => 6,
        }
    }
}

/// Message flags, as IMAP has them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
}

/// What the message list shows: the envelope, not the body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageHeader {
    pub id: String,
    pub folder_id: String,
    /// The id the server knows the message by (IMAP UID, EWS item id); empty for sample mail.
    pub remote_id: String,
    pub from: Address,
    pub to: Vec<Address>,
    pub subject: String,
    /// Seconds since the Unix epoch, UTC.
    pub date: i64,
    pub preview: String,
    pub flags: Flags,
    pub has_attachments: bool,
    /// The Message-ID header, for replies.
    pub message_id: Option<String>,
    /// The categories (coloured labels) on the message, by name; see [`Category`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// How urgent the sender said the message is.
    #[serde(default, skip_serializing_if = "Importance::is_normal")]
    pub importance: Importance,
    /// Set on a chat message (Delta Chat's chat over e-mail); see [`ChatTag`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<ChatTag>,
    /// Whether the message came encrypted with OpenPGP, and whether its header inside the
    /// encryption has been read yet.
    #[serde(default, skip_serializing_if = "Sealed::is_no")]
    pub sealed: Sealed,
}

/// An OpenPGP-encrypted message keeps its real header inside the encryption (protected
/// headers): Delta Chat shows only `[...]` as the subject outside, and a date up to a week
/// off. Once the message is opened, the header inside takes the outer one's place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sealed {
    /// Not encrypted with OpenPGP.
    #[default]
    No,
    /// Encrypted; the list shows the outer header.
    Closed,
    /// Encrypted, opened: the list shows the header from inside.
    Opened,
}

impl Sealed {
    pub fn is_no(&self) -> bool {
        *self == Sealed::No
    }
}

/// How urgent a message is, as the sender marked it: Outlook's "Hoge urgentie" and "Lage
/// urgentie". It travels as the `Importance` and `X-Priority` headers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    Low,
    #[default]
    Normal,
    High,
}

impl Importance {
    pub fn is_normal(&self) -> bool {
        *self == Importance::Normal
    }
}

/// A message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    /// Blind copies: they get the message, the others do not see them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<Address>,
    pub subject: String,
    pub body: String,
    /// The Message-ID this answers, if any.
    pub in_reply_to: Option<String>,
    /// Files going out with the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<DraftAttachment>,
    #[serde(default, skip_serializing_if = "Importance::is_normal")]
    pub importance: Importance,
    /// Asks the reader's mail program to say when the message was read, Outlook's
    /// "Leesbevestiging vragen"; goes out as the `Disposition-Notification-To` header.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_receipt: bool,
    /// Whether to sign the message with the account's S/MIME certificate; `None` signs when
    /// the account has one, the way Outlook's "Digitale handtekening toevoegen aan alle
    /// uitgaande berichten" does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign: Option<bool>,
    /// Encrypt the message (S/MIME) for its recipients and the sender, Outlook's "Versleutelen";
    /// it goes out only when every recipient's certificate is known.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub encrypt: bool,
    /// The formatting over `body`; with it the message goes out as HTML with the text beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich: Option<rich::Rich>,
    /// Sends the draft as a chat message: with `Chat-Version` (and the group's headers), and
    /// only end-to-end encrypted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<ChatTag>,
}

impl Draft {
    /// The size of the attachments together, before encoding.
    pub fn attachment_bytes(&self) -> usize {
        self.attachments.iter().map(|a| a.bytes.len()).sum()
    }
}

/// How much a message may carry in attachments, all together, before encoding. Most mail
/// servers refuse more.
pub const MAX_ATTACHMENTS: usize = 25 * 1024 * 1024;

/// A file going out with a draft.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftAttachment {
    /// The file name the recipient sees: no directories, no control characters.
    pub name: String,
    /// The media type, from the file's extension; `application/octet-stream` for the rest.
    pub mime: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

impl DraftAttachment {
    /// From a file name (a path is cut down to its last part) and the file's contents.
    pub fn new(name: &str, bytes: Vec<u8>) -> Self {
        let name = clean_text(name.rsplit(['/', '\\']).next().unwrap_or("").trim()).replace(['"', '\\', '/'], "_");
        let name = if name.is_empty() || name == "." || name == ".." { "bijlage".to_string() } else { name };
        let mime = mime_of(&name).to_string();
        DraftAttachment { name, mime, bytes }
    }
}

/// How long the text of a draft put aside may be, in characters.
pub const MAX_DRAFT_BODY: usize = 200_000;

/// A message being written that was put aside. The address lines are kept as typed, so an
/// unfinished one survives; the account's encrypted offline cache holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredDraft {
    /// Empty for one that has not been put aside yet; the engine then gives it one.
    #[serde(default)]
    pub id: String,
    /// Empty means the current account.
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bcc: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<DraftAttachment>,
    /// Seconds since the Unix epoch when it was last put aside.
    #[serde(default)]
    pub updated: i64,
    /// Set for a message that waits to go out at this moment (seconds since the Unix epoch),
    /// the way Outlook's "Bezorging uitstellen" keeps a message in the Outbox until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Importance::is_normal")]
    pub importance: Importance,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_receipt: bool,
    /// See [`Draft::sign`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign: Option<bool>,
    /// See [`Draft::encrypt`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub encrypt: bool,
    /// See [`Draft::rich`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich: Option<rich::Rich>,
}

impl StoredDraft {
    /// The message as it would go out: the address lines parsed, at least one recipient.
    pub fn to_draft(&self) -> Result<Draft, String> {
        let to = parse_address_list(&self.to).map_err(|_| "Controleer de adressen bij Aan.".to_string())?;
        let cc = parse_address_list(&self.cc).map_err(|_| "Controleer de adressen bij CC.".to_string())?;
        let bcc = parse_address_list(&self.bcc).map_err(|_| "Controleer de adressen bij BCC.".to_string())?;
        if to.is_empty() {
            return Err("Vul ten minste één ontvanger in bij Aan.".into());
        }
        let rich = self.rich.clone().map(|mut r| {
            r.fit(&self.body);
            r
        });
        Ok(Draft {
            to,
            cc,
            bcc,
            subject: self.subject.clone(),
            body: self.body.clone(),
            in_reply_to: self.in_reply_to.clone(),
            attachments: self.attachments.clone(),
            importance: self.importance,
            read_receipt: self.read_receipt,
            sign: self.sign,
            encrypt: self.encrypt,
            rich: rich.filter(|r| !r.is_plain()),
            chat: None,
        })
    }

    /// Cleans the free text so it is safe to store and show; the body keeps its line breaks.
    pub fn cleaned(mut self) -> Self {
        self.to = clean_text(self.to.trim());
        self.cc = clean_text(self.cc.trim());
        self.bcc = clean_text(self.bcc.trim());
        self.subject = clean_text(self.subject.trim());
        self.body = self.body.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(MAX_DRAFT_BODY).collect();
        if let Some(r) = &mut self.rich {
            r.fit(&self.body);
        }
        self
    }

    /// Whether there is nothing in it worth keeping.
    pub fn is_blank(&self) -> bool {
        self.to.trim().is_empty()
            && self.cc.trim().is_empty()
            && self.bcc.trim().is_empty()
            && self.subject.trim().is_empty()
            && self.body.trim().is_empty()
            && self.attachments.is_empty()
    }
}

/// The media type that goes with a file name's extension.
pub fn mime_of(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "txt" | "md" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "ics" => "text/calendar",
        "vcf" => "text/vcard",
        "eml" => "message/rfc822",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "rtf" => "application/rtf",
        _ => "application/octet-stream",
    }
}

/// Attachment contents as base64 in JSON, a quarter the size of a list of numbers.
mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

/// The prefixes a reply or forward puts before a subject, in the languages mail clients use.
const SUBJECT_PREFIXES: [&str; 9] = ["re:", "fwd:", "fw:", "antw:", "aw:", "doorst:", "tr:", "wg:", "sv:"];

/// The subject without its reply and forward prefixes ("Re:", "Fwd:", "Antw:" and the like,
/// however many and in any case), trimmed and lower-cased. Messages with the same key belong to
/// one conversation.
pub fn conversation_key(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        let Some(prefix) = SUBJECT_PREFIXES.iter().find(|p| lower.starts_with(**p)) else { break };
        // The prefix is ASCII, so its length is the same in `s` and in `lower`.
        s = s.get(prefix.len()..).unwrap_or("").trim_start();
    }
    s.trim().to_lowercase()
}

/// A conversation the person ignores (Outlook's "Negeren"): new mail with this key in the
/// account's inbox goes straight to Deleted Items. `subject` is the subject as it was, for
/// showing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredConversation {
    pub account_id: String,
    pub key: String,
    #[serde(default)]
    pub subject: String,
}

/// A snoozed message, Outlook's "Sluimeren": out of the list until `until` (Unix seconds),
/// then back as unread. Filed under `key` (the message's Message-ID, else its id) so a move
/// between folders keeps it; `subject` is the subject as it was, for showing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snooze {
    pub key: String,
    pub until: i64,
    #[serde(default)]
    pub subject: String,
}

/// A follow-up flag with a date, Outlook's "Opvolgen": the day a flagged message is due. The
/// flag itself is on the server; the date is this computer's, filed under `key` (the
/// message's Message-ID, else its id) so a move between folders keeps it. `due` is an ISO
/// day, "2026-10-21"; `subject` is the subject as it was, for showing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowUp {
    pub key: String,
    pub due: String,
    #[serde(default)]
    pub subject: String,
    /// Whether the reminder on the due day was closed; a new day opens it again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dismissed: bool,
}

/// `s` cut to [`MAX_TEXT`] characters with control characters (other than tab) removed, so a
/// header cannot smuggle line breaks or terminal escapes into the UI or a log.
pub fn clean_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\t').take(MAX_TEXT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_key_drops_every_reply_and_forward_prefix() {
        assert_eq!(conversation_key("Re: RE: Fwd: Offerte drukwerk "), "offerte drukwerk");
        assert_eq!(conversation_key("Antw: Planning"), conversation_key("planning"));
        assert_eq!(conversation_key("FW:Doorst: Reünie"), "reünie");
        assert_eq!(conversation_key("Rente"), "rente", "only a prefix with its colon counts");
        assert_eq!(conversation_key("Re:"), "");
        assert_eq!(conversation_key(""), "");
    }

    #[test]
    fn clean_text_drops_controls_and_bounds_length() {
        assert_eq!(clean_text("Hi\r\nBcc: x\u{1b}[31m"), "HiBcc: x[31m");
        assert_eq!(clean_text(&"a".repeat(5000)).chars().count(), MAX_TEXT);
    }
}
