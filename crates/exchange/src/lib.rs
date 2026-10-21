//! Exchange Web Services (EWS) for Exchange Server 2013 and later, including Exchange Online.
//!
//! SOAP over HTTPS, nothing else: the URL is always `https://<host>/EWS/Exchange.asmx`, the
//! certificate is checked by the platform verifier, and the request carries either HTTP Basic
//! credentials or an OAuth Bearer token. Every value that goes into a request is XML-escaped,
//! and responses are read with a size cap and a read-only parser.
//!
//! What is used: `GetFolder` and `FindFolder` for the folder list, `FindItem` for the message
//! list, `GetItem` with `IncludeMimeContent` for bodies (parsed by the same MIME code as IMAP
//! mail), `UpdateItem` for read and flagged, `MoveItem`, and `CreateItem` to send and file.
//! The calendar, contacts and tasks live in [`items`].

mod items;

pub use items::windows_zone;

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use merel_mailbox::{Credential, Flag, Mailbox, MailboxError, RemoteFolder, RemoteHeader, Result, kind_from_name};
use merel_model::{Address, FolderKind, Importance, Server, clean_text};
use merel_secrets::Secret;
use roxmltree::{Document, Node};

/// The largest response accepted: a body of `merel_mime::MAX_BODY` in base64, with room.
const MAX_RESPONSE: u64 = 32 * 1024 * 1024;
/// Folders returned per `FindFolder` page; a mailbox with more just shows the first ones.
const MAX_FOLDERS: u32 = 1000;

const NS_T: &str = "http://schemas.microsoft.com/exchange/services/2006/types";
const NS_M: &str = "http://schemas.microsoft.com/exchange/services/2006/messages";
const NS_SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";

/// MAPI property tags read and written as extended properties.
const PR_FLAG_STATUS: &str = "0x1090";
const PR_LAST_VERB_EXECUTED: &str = "0x1081";
const PR_MESSAGE_FLAGS: &str = "0x0E07";

/// The well-known folders asked for by name, with the kind each one is.
const DISTINGUISHED: [(&str, FolderKind); 5] = [
    ("inbox", FolderKind::Inbox),
    ("drafts", FolderKind::Drafts),
    ("sentitems", FolderKind::Sent),
    ("deleteditems", FolderKind::Deleted),
    ("junkemail", FolderKind::Junk),
];

pub struct EwsClient {
    agent: ureq::Agent,
    url: String,
    host: String,
    /// The whole `Authorization` header value; it holds the credentials, so it is a secret.
    authorization: Secret,
    /// Folder ids of the well-known folders, learned at connect.
    known: Vec<(String, FolderKind)>,
    /// The computer's time zone as Windows names it, when known; it tells the server which
    /// midnight an all-day appointment or a due day means.
    time_zone: Option<&'static str>,
}

/// `s` with the five XML special characters escaped, for element text and attribute values.
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

fn envelope(body: &str, time_zone: Option<&str>) -> String {
    let zone = time_zone.map(|z| format!("<t:TimeZoneContext><t:TimeZoneDefinition Id=\"{}\"/></t:TimeZoneContext>", esc(z))).unwrap_or_default();
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            "<soap:Envelope xmlns:soap=\"{soap}\" xmlns:t=\"{t}\" xmlns:m=\"{m}\">",
            "<soap:Header><t:RequestServerVersion Version=\"Exchange2013\"/>{zone}</soap:Header>",
            "<soap:Body>{body}</soap:Body></soap:Envelope>"
        ),
        soap = NS_SOAP,
        t = NS_T,
        m = NS_M,
        zone = zone,
        body = body
    )
}

/// The element named `name` (in any namespace) among `node`'s descendants.
pub(crate) fn find<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.descendants().find(|n| n.is_element() && n.tag_name().name() == name)
}

/// The direct child element named `name`.
pub(crate) fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|n| n.is_element() && n.tag_name().name() == name)
}

pub(crate) fn text<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    child(node, name).and_then(|n| n.text()).map(str::trim)
}

pub(crate) fn parse(xml: &str) -> Result<Document<'_>> {
    Document::parse(xml).map_err(|e| MailboxError::Protocol(format!("unreadable reply: {e}")))
}

/// The `ResponseMessages` children of a reply, each checked for `ResponseClass="Error"`
/// unless `lenient`, in which case failed ones come back as `None`.
pub(crate) fn response_messages<'a>(doc: &'a Document<'a>, lenient: bool) -> Result<Vec<Option<Node<'a, 'a>>>> {
    let root = doc.root_element();
    if let Some(fault) = find(root, "Fault") {
        let reason = text(fault, "faultstring").unwrap_or("SOAP fault");
        return Err(MailboxError::Protocol(clip(reason)));
    }
    let messages = find(root, "ResponseMessages").ok_or_else(|| MailboxError::Protocol("no response messages".into()))?;
    let mut out = Vec::new();
    for m in messages.children().filter(|n| n.is_element()) {
        if m.attribute("ResponseClass") == Some("Error") {
            if lenient {
                out.push(None);
                continue;
            }
            let code = text(m, "ResponseCode").unwrap_or("error");
            let what = text(m, "MessageText").unwrap_or("");
            return Err(MailboxError::Protocol(clip(&format!("{code}: {what}"))));
        }
        out.push(Some(m));
    }
    Ok(out)
}

pub(crate) fn clip(s: &str) -> String {
    clean_text(s).chars().take(200).collect()
}

/// `2026-10-18T21:45:20Z` (or with fractional seconds) as seconds since the Unix epoch.
pub(crate) fn parse_datetime(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let time = time.split(['+', '-']).next()?;
    let mut t = time.split(':').map(|p| p.split('.').next().and_then(|p| p.parse::<i64>().ok()));
    let (h, mi, se) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    // Days from civil, after Howard Hinnant.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se)
}

fn mailbox_address(node: Node<'_, '_>) -> Option<Address> {
    let m = child(node, "Mailbox")?;
    let email = text(m, "EmailAddress")?;
    let name = text(m, "Name").filter(|n| !n.is_empty() && *n != email);
    Address::new(name, email).ok()
}

/// The value of the extended property with `tag` on an item, if the server sent it.
fn extended<'a>(item: Node<'a, 'a>, tag: &str) -> Option<&'a str> {
    item.children().filter(|n| n.is_element() && n.tag_name().name() == "ExtendedProperty").find_map(|p| {
        let uri = child(p, "ExtendedFieldURI")?;
        uri.attribute("PropertyTag").filter(|t| t.eq_ignore_ascii_case(tag))?;
        text(p, "Value")
    })
}

fn extended_field(tag: &str) -> String {
    format!("<t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"Integer\"/>")
}

impl EwsClient {
    /// Builds the client and checks the credentials with one small request.
    pub fn connect(server: &Server, username: &str, credential: &Credential) -> Result<Self> {
        let host = server.host.trim().to_ascii_lowercase();
        if host.is_empty() || host.chars().any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))) {
            return Err(MailboxError::BadName(host));
        }
        let url = if server.port == 443 {
            format!("https://{host}/EWS/Exchange.asmx")
        } else {
            format!("https://{host}:{}/EWS/Exchange.asmx", server.port)
        };
        let authorization = match credential {
            Credential::Password(p) => Secret::new(format!("Basic {}", BASE64.encode(format!("{username}:{}", p.expose())))),
            Credential::OAuth2(t) => Secret::new(format!("Bearer {}", t.expose())),
        };
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .tls_config(tls())
                .timeout_global(Some(Duration::from_secs(60)))
                .http_status_as_error(false)
                .user_agent("Merel")
                .build(),
        );
        let time_zone = iana_time_zone::get_timezone().ok().as_deref().and_then(windows_zone);
        let mut client = EwsClient { agent, url, host, authorization, known: Vec::new(), time_zone };
        client.known = client.well_known()?;
        Ok(client)
    }

    /// A client that never connects, for tests of the requests it builds.
    #[cfg(test)]
    pub(crate) fn offline(time_zone: Option<&'static str>) -> Self {
        EwsClient {
            agent: ureq::Agent::new_with_config(ureq::config::Config::builder().tls_config(tls()).build()),
            url: String::new(),
            host: "test".into(),
            authorization: Secret::new(String::new()),
            known: Vec::new(),
            time_zone,
        }
    }

    /// Posts one SOAP body and returns the reply's XML.
    pub(crate) fn call(&self, body: &str) -> Result<String> {
        let xml = envelope(body, self.time_zone);
        let mut response = self
            .agent
            .post(&self.url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("Authorization", self.authorization.expose())
            .send(xml.as_bytes())
            .map_err(|e| MailboxError::Connect { host: self.host.clone(), reason: e.to_string() })?;
        let status = response.status().as_u16();
        if status == 401 {
            let schemes: Vec<String> =
                response.headers().get_all("www-authenticate").iter().filter_map(|v| v.to_str().ok()).map(str::to_string).collect();
            let basic = schemes.iter().any(|s| s.to_ascii_lowercase().contains("basic") || s.to_ascii_lowercase().contains("bearer"));
            if !basic && !schemes.is_empty() {
                let offered = schemes.iter().map(|s| s.split_whitespace().next().unwrap_or("")).collect::<Vec<_>>().join("/");
                return Err(MailboxError::AuthScheme(clip(&offered)));
            }
            return Err(MailboxError::Login);
        }
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE)
            .read_to_string()
            .map_err(|e| MailboxError::Protocol(format!("could not read the reply: {e}")))?;
        if !(200..300).contains(&status) && !text.trim_start().starts_with('<') {
            return Err(MailboxError::Protocol(format!("HTTP {status}")));
        }
        Ok(text)
    }

    /// The ids of the well-known folders, in `DISTINGUISHED` order; a missing one is skipped.
    fn well_known(&self) -> Result<Vec<(String, FolderKind)>> {
        let ids: String = DISTINGUISHED.iter().map(|(name, _)| format!("<t:DistinguishedFolderId Id=\"{name}\"/>")).collect();
        let xml = self.call(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"
        ))?;
        let doc = parse(&xml)?;
        let messages = response_messages(&doc, true)?;
        if messages.iter().all(Option::is_none) {
            return Err(MailboxError::Protocol("no folders".into()));
        }
        Ok(messages
            .into_iter()
            .zip(DISTINGUISHED)
            .filter_map(|(m, (_, kind))| {
                let id = find(m?, "FolderId")?.attribute("Id")?;
                Some((id.to_string(), kind))
            })
            .collect())
    }

    fn kind_of(&self, id: &str, display_name: &str) -> FolderKind {
        self.known.iter().find(|(k, _)| k == id).map(|(_, kind)| *kind).unwrap_or_else(|| match kind_from_name(display_name) {
            // Only the server knows which folder really is the inbox or the junk folder.
            FolderKind::Inbox | FolderKind::Junk | FolderKind::Deleted | FolderKind::Sent | FolderKind::Drafts => FolderKind::Other,
            k => k,
        })
    }

    /// The current change key of a folder, which `UpdateFolder` insists on.
    fn folder_change_key(&self, id: &str) -> Result<String> {
        let xml = self.call(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:GetFolder>",
            esc(id)
        ))?;
        let doc = parse(&xml)?;
        response_messages(&doc, false)?;
        find(doc.root_element(), "FolderId")
            .and_then(|n| n.attribute("ChangeKey"))
            .map(str::to_string)
            .ok_or_else(|| MailboxError::Protocol("no change key".into()))
    }

    /// The current change key of an item, which `UpdateItem` insists on.
    fn change_key(&self, id: &str) -> Result<String> {
        let xml = self.call(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        let doc = parse(&xml)?;
        response_messages(&doc, false)?;
        find(doc.root_element(), "ItemId")
            .and_then(|n| n.attribute("ChangeKey"))
            .map(str::to_string)
            .ok_or_else(|| MailboxError::Protocol("no change key".into()))
    }

    pub(crate) fn update(&self, id: &str, change: &str) -> Result<()> {
        let key = self.change_key(id)?;
        let xml = self.call(&format!(
            concat!(
                "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\" ",
                "SendMeetingInvitationsOrCancellations=\"SendToNone\"><m:ItemChanges><t:ItemChange>",
                "<t:ItemId Id=\"{id}\" ChangeKey=\"{key}\"/><t:Updates>{change}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"
            ),
            id = esc(id),
            key = esc(&key),
            change = change
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }

    /// Creates a message from raw MIME in `folder` (a folder id or a distinguished name).
    fn create(&self, disposition: &str, folder_xml: &str, message: &[u8], extra: &str) -> Result<()> {
        let xml = self.call(&format!(
            concat!(
                "<m:CreateItem MessageDisposition=\"{disposition}\"><m:SavedItemFolderId>{folder}</m:SavedItemFolderId>",
                "<m:Items><t:Message><t:MimeContent CharacterSet=\"UTF-8\">{mime}</t:MimeContent>{extra}</t:Message></m:Items></m:CreateItem>"
            ),
            disposition = disposition,
            folder = folder_xml,
            mime = BASE64.encode(message),
            extra = extra
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }
}

/// The `Categories` strings of an item.
pub(crate) fn categories_of(item: Node<'_, '_>) -> Vec<String> {
    child(item, "Categories")
        .map(|c| c.children().filter(|n| n.is_element()).filter_map(|n| n.text()).map(clean_text).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

/// The item's `Importance` element: `High`, `Normal` or `Low`.
fn importance_of(item: Node<'_, '_>) -> Importance {
    match text(item, "Importance") {
        Some("High") => Importance::High,
        Some("Low") => Importance::Low,
        _ => Importance::Normal,
    }
}

fn header_of(item: Node<'_, '_>) -> RemoteHeader {
    let subject = text(item, "Subject").map(clean_text).unwrap_or_default();
    let date = text(item, "DateTimeReceived").and_then(parse_datetime).unwrap_or(0);
    let message_id = text(item, "InternetMessageId").map(clean_text).filter(|m| !m.is_empty());
    let from = child(item, "From").and_then(mailbox_address);
    let last_verb = extended(item, PR_LAST_VERB_EXECUTED).and_then(|v| v.parse::<u32>().ok());
    RemoteHeader {
        id: child(item, "ItemId").and_then(|n| n.attribute("Id")).unwrap_or("").to_string(),
        seen: text(item, "IsRead") == Some("true"),
        flagged: extended(item, PR_FLAG_STATUS) == Some("2"),
        answered: matches!(last_verb, Some(102 | 103)),
        size: text(item, "Size").and_then(|s| s.parse().ok()).unwrap_or(0),
        has_attachments: text(item, "HasAttachments") == Some("true"),
        categories: categories_of(item),
        header: merel_mime::ParsedHeader { from, to: Vec::new(), subject, date: Some(date), message_id, importance: importance_of(item) },
        date,
    }
}

/// A `FindItem` for one page of a folder, newest first; with `query`, only the messages
/// Exchange's search finds for it (every word required, as the server interprets it).
fn find_item_request(folder: &str, offset: u32, limit: u32, query: Option<&str>) -> String {
    let query = query.map(|q| format!("<m:QueryString>{}</m:QueryString>", esc(q))).unwrap_or_default();
    format!(
        concat!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>",
            "<t:FieldURI FieldURI=\"item:Subject\"/><t:FieldURI FieldURI=\"item:DateTimeReceived\"/>",
            "<t:FieldURI FieldURI=\"item:HasAttachments\"/><t:FieldURI FieldURI=\"item:Size\"/><t:FieldURI FieldURI=\"item:Categories\"/>",
            "<t:FieldURI FieldURI=\"item:Importance\"/>",
            "<t:FieldURI FieldURI=\"message:From\"/><t:FieldURI FieldURI=\"message:IsRead\"/>",
            "<t:FieldURI FieldURI=\"message:InternetMessageId\"/>{flag}{verb}",
            "</t:AdditionalProperties></m:ItemShape>",
            "<m:IndexedPageItemView MaxEntriesReturned=\"{limit}\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>",
            "<m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>",
            "<m:ParentFolderIds><t:FolderId Id=\"{folder}\"/></m:ParentFolderIds>{query}</m:FindItem>"
        ),
        flag = extended_field(PR_FLAG_STATUS),
        verb = extended_field(PR_LAST_VERB_EXECUTED),
        limit = limit.clamp(1, 1000),
        offset = offset,
        folder = esc(folder),
        query = query
    )
}

impl EwsClient {
    /// Runs a `FindItem` and reads the messages out of it, newest last.
    fn find_items(&mut self, request: &str) -> Result<Vec<RemoteHeader>> {
        let xml = self.call(request)?;
        let doc = parse(&xml)?;
        response_messages(&doc, false)?;
        let Some(items) = find(doc.root_element(), "Items") else { return Ok(Vec::new()) };
        let mut out: Vec<RemoteHeader> = items.children().filter(|n| n.is_element()).map(header_of).filter(|h| !h.id.is_empty()).collect();
        out.reverse();
        Ok(out)
    }
}

impl Mailbox for EwsClient {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        let xml = self.call(&format!(
            concat!(
                "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>",
                "<t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:FolderClass\"/>",
                "</t:AdditionalProperties></m:FolderShape>",
                "<m:IndexedPageFolderView MaxEntriesReturned=\"{max}\" Offset=\"0\" BasePoint=\"Beginning\"/>",
                "<m:ParentFolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\"/></m:ParentFolderIds></m:FindFolder>"
            ),
            max = MAX_FOLDERS
        ))?;
        let doc = parse(&xml)?;
        response_messages(&doc, false)?;
        let Some(list) = find(doc.root_element(), "Folders") else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for f in list.children().filter(|n| n.is_element()) {
            let Some(id) = child(f, "FolderId").and_then(|n| n.attribute("Id")) else { continue };
            // Calendars, contacts and tasks are folders too; only mail folders belong here.
            if text(f, "FolderClass").is_some_and(|c| !c.starts_with("IPF.Note") && !c.is_empty()) {
                continue;
            }
            let name = clean_text(text(f, "DisplayName").unwrap_or("Map"));
            out.push(RemoteFolder { path: id.to_string(), kind: self.kind_of(id, &name), name, selectable: true });
        }
        if !out.iter().any(|f| f.kind == FolderKind::Inbox) {
            return Err(MailboxError::Protocol("the mailbox has no inbox".into()));
        }
        Ok(out)
    }

    fn headers(&mut self, folder: &str, offset: u32, limit: u32) -> Result<Vec<RemoteHeader>> {
        self.find_items(&find_item_request(folder, offset, limit, None))
    }

    fn search(&mut self, folder: &str, query: &str, limit: u32) -> Result<Vec<RemoteHeader>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        self.find_items(&find_item_request(folder, 0, limit, Some(query)))
    }

    fn body(&mut self, _folder: &str, id: &str) -> Result<Vec<u8>> {
        let xml = self.call(&format!(
            concat!(
                "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape>",
                "<m:ItemIds><t:ItemId Id=\"{id}\"/></m:ItemIds></m:GetItem>"
            ),
            id = esc(id)
        ))?;
        let doc = parse(&xml)?;
        response_messages(&doc, false)?;
        let mime = find(doc.root_element(), "MimeContent").and_then(|n| n.text()).unwrap_or("");
        let compact: String = mime.chars().filter(|c| !c.is_whitespace()).collect();
        BASE64.decode(compact).map_err(|_| MailboxError::Protocol("the message content is not valid base64".into()))
    }

    fn set_flag(&mut self, _folder: &str, id: &str, flag: Flag, on: bool) -> Result<()> {
        let change = match (flag, on) {
            (Flag::Seen, on) => {
                format!("<t:SetItemField><t:FieldURI FieldURI=\"message:IsRead\"/><t:Message><t:IsRead>{on}</t:IsRead></t:Message></t:SetItemField>")
            }
            (Flag::Flagged, true) => format!(
                "<t:SetItemField>{f}<t:Message><t:ExtendedProperty>{f}<t:Value>2</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>",
                f = extended_field(PR_FLAG_STATUS)
            ),
            (Flag::Flagged, false) => format!("<t:DeleteItemField>{}</t:DeleteItemField>", extended_field(PR_FLAG_STATUS)),
        };
        self.update(id, &change)
    }

    fn move_to(&mut self, _folder: &str, id: &str, dest: &str) -> Result<()> {
        let xml = self.call(&format!(
            "<m:MoveItem><m:ToFolderId><t:FolderId Id=\"{dest}\"/></m:ToFolderId><m:ItemIds><t:ItemId Id=\"{id}\"/></m:ItemIds></m:MoveItem>",
            dest = esc(dest),
            id = esc(id)
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }

    fn mark_junk(&mut self, _folder: &str, id: &str, junk: bool) -> Result<()> {
        // Exchange 2013 and later: the sender joins (or leaves) the mailbox's blocked senders,
        // which Outlook and the phone see too. The move is a separate step, so the item stays.
        let xml = self.call(&format!(
            "<m:MarkAsJunk IsJunk=\"{junk}\" MoveItem=\"false\"><m:ItemIds><t:ItemId Id=\"{id}\"/></m:ItemIds></m:MarkAsJunk>",
            id = esc(id)
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }

    fn create_folder(&mut self, parent: Option<&str>, name: &str) -> Result<()> {
        let parent = match parent {
            Some(id) => format!("<t:FolderId Id=\"{}\"/>", esc(id)),
            None => "<t:DistinguishedFolderId Id=\"msgfolderroot\"/>".to_string(),
        };
        let xml = self.call(&format!(
            concat!(
                "<m:CreateFolder><m:ParentFolderId>{parent}</m:ParentFolderId>",
                "<m:Folders><t:Folder><t:FolderClass>IPF.Note</t:FolderClass><t:DisplayName>{name}</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>"
            ),
            parent = parent,
            name = esc(name)
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }

    fn rename_folder(&mut self, folder: &str, name: &str) -> Result<()> {
        let key = self.folder_change_key(folder)?;
        let xml = self.call(&format!(
            concat!(
                "<m:UpdateFolder><m:FolderChanges><t:FolderChange><t:FolderId Id=\"{id}\" ChangeKey=\"{key}\"/><t:Updates>",
                "<t:SetFolderField><t:FieldURI FieldURI=\"folder:DisplayName\"/><t:Folder><t:DisplayName>{name}</t:DisplayName></t:Folder></t:SetFolderField>",
                "</t:Updates></t:FolderChange></m:FolderChanges></m:UpdateFolder>"
            ),
            id = esc(folder),
            key = esc(&key),
            name = esc(name)
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }

    fn delete_folder(&mut self, folder: &str) -> Result<()> {
        // Soft: the folder leaves the mailbox but the server can still recover it for a while.
        let xml = self.call(&format!(
            "<m:DeleteFolder DeleteType=\"SoftDelete\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:DeleteFolder>",
            esc(folder)
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }

    fn delete_message(&mut self, _folder: &str, id: &str) -> Result<()> {
        // Soft: out of the mailbox, but the server can still recover it for a while.
        let xml =
            self.call(&format!("<m:DeleteItem DeleteType=\"SoftDelete\"><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>", esc(id)))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }

    fn set_categories(&mut self, _folder: &str, id: &str, categories: &[String], _removed: &[String]) -> Result<()> {
        let change = if categories.is_empty() {
            "<t:DeleteItemField><t:FieldURI FieldURI=\"item:Categories\"/></t:DeleteItemField>".to_string()
        } else {
            let strings: String = categories.iter().map(|c| format!("<t:String>{}</t:String>", esc(c))).collect();
            format!(
                "<t:SetItemField><t:FieldURI FieldURI=\"item:Categories\"/><t:Message><t:Categories>{strings}</t:Categories></t:Message></t:SetItemField>"
            )
        };
        self.update(id, &change)
    }

    fn empty_folder(&mut self, folder: &str) -> Result<()> {
        let xml = self.call(&format!(
            "<m:EmptyFolder DeleteType=\"SoftDelete\" DeleteSubFolders=\"false\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:EmptyFolder>",
            esc(folder)
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }

    fn append(&mut self, folder: &str, message: &[u8]) -> Result<()> {
        // MSGFLAG_READ, so the copy is neither unread nor a draft.
        let read = format!("<t:ExtendedProperty>{}<t:Value>1</t:Value></t:ExtendedProperty>", extended_field(PR_MESSAGE_FLAGS));
        self.create("SaveOnly", &format!("<t:FolderId Id=\"{}\"/>", esc(folder)), message, &read)
    }

    fn send(&mut self, message: &[u8]) -> Result<bool> {
        self.create("SendAndSaveCopy", "<t:DistinguishedFolderId Id=\"sentitems\"/>", message, "")?;
        Ok(true)
    }

    fn items(&mut self) -> Option<&mut dyn merel_mailbox::ItemStore> {
        Some(self)
    }

    fn logout(self: Box<Self>) {}
}

/// HTTPS checked by the computer's own certificate store (Keychain on macOS), with aws-lc-rs
/// like the rest of Merel. ureq has no root certificates of its own in this build: an
/// agent without this panics on its first request.
fn tls() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_is_a_find_item_with_a_query_string() {
        let plain = find_item_request("AAMk", 200, 50, None);
        assert!(plain.contains("Offset=\"200\"") && plain.contains("MaxEntriesReturned=\"50\"") && !plain.contains("QueryString"));
        let search = find_item_request("AAMk", 0, 5000, Some("factuur <2026>"));
        assert!(search.ends_with("</m:ParentFolderIds><m:QueryString>factuur &lt;2026&gt;</m:QueryString></m:FindItem>"), "{search}");
        assert!(search.contains("MaxEntriesReturned=\"1000\""), "the page size is capped");
    }

    #[test]
    fn escaping_covers_the_special_characters_and_drops_controls() {
        assert_eq!(esc("a<b>&\"c'\u{1}"), "a&lt;b&gt;&amp;&quot;c&apos;");
        assert!(envelope("<m:X/>", None).contains("<soap:Body><m:X/></soap:Body>"));
        assert!(envelope("", Some("W. Europe Standard Time")).contains("<t:TimeZoneDefinition Id=\"W. Europe Standard Time\"/>"));
    }

    #[test]
    fn datetimes_parse_to_unix_seconds() {
        assert_eq!(parse_datetime("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_datetime("2026-10-14T17:46:40Z"), Some(1_792_000_000));
        assert_eq!(parse_datetime("2026-10-14T17:46:40.123Z"), Some(1_792_000_000));
        assert_eq!(parse_datetime("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_datetime("garbage"), None);
    }

    fn reply(messages: &str) -> String {
        format!(
            "<s:Envelope xmlns:s=\"{NS_SOAP}\"><s:Body><m:FindItemResponse xmlns:m=\"{NS_M}\" xmlns:t=\"{NS_T}\"><m:ResponseMessages>{messages}</m:ResponseMessages></m:FindItemResponse></s:Body></s:Envelope>"
        )
    }

    #[test]
    fn error_replies_become_protocol_errors() {
        let xml = reply(
            "<m:FindItemResponseMessage ResponseClass=\"Error\"><m:MessageText>No such folder.</m:MessageText><m:ResponseCode>ErrorFolderNotFound</m:ResponseCode></m:FindItemResponseMessage>",
        );
        let doc = parse(&xml).unwrap();
        match response_messages(&doc, false) {
            Err(MailboxError::Protocol(m)) => assert_eq!(m, "ErrorFolderNotFound: No such folder."),
            other => panic!("{other:?}"),
        }
        assert_eq!(response_messages(&doc, true).unwrap(), vec![None]);
        let fault =
            format!("<s:Envelope xmlns:s=\"{NS_SOAP}\"><s:Body><s:Fault><faultstring>Bad version</faultstring></s:Fault></s:Body></s:Envelope>");
        assert!(matches!(response_messages(&parse(&fault).unwrap(), true), Err(MailboxError::Protocol(m)) if m == "Bad version"));
    }

    #[test]
    fn items_become_headers() {
        let xml = reply(concat!(
            "<m:FindItemResponseMessage ResponseClass=\"Success\"><m:RootFolder><t:Items>",
            "<t:Message><t:ItemId Id=\"AAMk1\" ChangeKey=\"CQAA\"/><t:Subject>Hoi &amp; dag</t:Subject>",
            "<t:DateTimeReceived>2026-10-14T17:46:40Z</t:DateTimeReceived><t:HasAttachments>true</t:HasAttachments><t:Size>1234</t:Size>",
            "<t:From><t:Mailbox><t:Name>Sanne</t:Name><t:EmailAddress>sanne@example.org</t:EmailAddress></t:Mailbox></t:From>",
            "<t:IsRead>false</t:IsRead><t:InternetMessageId>&lt;abc@example.org&gt;</t:InternetMessageId><t:Importance>High</t:Importance>",
            "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty>",
            "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1081\" PropertyType=\"Integer\"/><t:Value>102</t:Value></t:ExtendedProperty>",
            "</t:Message></t:Items></m:RootFolder></m:FindItemResponseMessage>"
        ));
        let doc = parse(&xml).unwrap();
        response_messages(&doc, false).unwrap();
        let items = find(doc.root_element(), "Items").unwrap();
        let h = items.children().filter(|n| n.is_element()).map(header_of).next().unwrap();
        assert_eq!(h.id, "AAMk1");
        assert_eq!(h.header.subject, "Hoi & dag");
        assert_eq!(h.date, 1_792_000_000);
        assert!(h.has_attachments && h.flagged && h.answered && !h.seen);
        assert_eq!(h.size, 1234);
        let from = h.header.from.unwrap();
        assert_eq!((from.display(), from.email()), ("Sanne", "sanne@example.org"));
        assert_eq!(h.header.message_id.as_deref(), Some("<abc@example.org>"));
        assert_eq!(h.header.importance, Importance::High);
    }

    #[test]
    fn connect_refuses_a_bad_host_before_touching_the_network() {
        let server = Server { host: "evil host/..".into(), port: 443, tls: merel_model::Tls::Implicit };
        assert!(matches!(EwsClient::connect(&server, "u", &Credential::Password(Secret::new("p"))), Err(MailboxError::BadName(_))));
    }
}
