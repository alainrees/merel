//! IMAP accounts with a CalDAV/CardDAV server next to them (Fastmail, iCloud, Nextcloud,
//! Posteo, mailbox.org and most self-hosted mail): the mail stays IMAP, and the server's
//! calendars, address books and task lists are an [`ItemStore`] behind the same user name and
//! password.
//!
//! Discovery follows RFC 6764: `/.well-known/caldav` (and `carddav`) on the server the person
//! named, then `current-user-principal`, the home sets and the collections in them. Every
//! request is HTTPS with the platform certificate verifier and Basic authentication; redirects
//! are followed by hand so the method and body stay what they were; replies are size-capped
//! and read with a read-only XML parser. Items are plain iCalendar and vCard resources
//! ([`ical`]); a change is written back into the resource as it was read, so what Mailcraft
//! does not model (alarms, attendees, photos) stays. Recurring appointments are shown as the
//! server expands them and are not edited from here.
//!
//! Store ids are `e:<url>`, `c:<url>` and `t:<url>`: the resource's own address, which is
//! what reads, changes and deletes it. An instance of a recurring appointment carries its
//! `RECURRENCE-ID` after a `#`, which marks it read-only.

pub mod ical;

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use mailcraft_imap::ImapClient;
use mailcraft_mailbox::{Credential, Flag, Item, ItemStore, Mailbox, MailboxError, RemoteFolder, RemoteHeader, Result};
use mailcraft_model::{Appointment, Contact, Dav, Server, Task, clean_text};
use mailcraft_secrets::Secret;
use roxmltree::{Document, Node};

use ical::Component;

const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
/// The largest reply accepted.
const MAX_REPLY: u64 = 16 * 1024 * 1024;
/// Redirects followed for one request.
const MAX_REDIRECTS: usize = 5;
/// Cards fetched per `addressbook-multiget`.
const CARD_BATCH: usize = 50;

/// An IMAP account's store: IMAP for mail, DAV for the rest.
pub struct DavMailbox {
    imap: ImapClient,
    items: DavClient,
}

impl DavMailbox {
    /// Logs in over IMAP and finds the calendars and address books on the DAV server, with the
    /// same user name and password. A DAV server that cannot be reached or holds no calendar
    /// is an error, since the person asked for it.
    pub fn connect(server: &Server, username: &str, credential: &Credential, dav: &Dav) -> Result<Self> {
        let password = match credential {
            Credential::Password(p) => p.clone(),
            Credential::OAuth2(_) => {
                return Err(MailboxError::Protocol("CalDAV/CardDAV werkt met een wachtwoord, niet met een browser-aanmelding".into()));
            }
        };
        let imap = ImapClient::connect(server, username, credential).map_err(MailboxError::from)?;
        let items = DavClient::connect(dav, username, password)?;
        Ok(DavMailbox { imap, items })
    }
}

impl Mailbox for DavMailbox {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        Mailbox::folders(&mut self.imap)
    }

    fn headers(&mut self, folder: &str, offset: u32, limit: u32) -> Result<Vec<RemoteHeader>> {
        Mailbox::headers(&mut self.imap, folder, offset, limit)
    }

    fn search(&mut self, folder: &str, query: &str, limit: u32) -> Result<Vec<RemoteHeader>> {
        Mailbox::search(&mut self.imap, folder, query, limit)
    }

    fn body(&mut self, folder: &str, id: &str) -> Result<Vec<u8>> {
        Mailbox::body(&mut self.imap, folder, id)
    }

    fn set_flag(&mut self, folder: &str, id: &str, flag: Flag, on: bool) -> Result<()> {
        Mailbox::set_flag(&mut self.imap, folder, id, flag, on)
    }

    fn move_to(&mut self, folder: &str, id: &str, dest: &str) -> Result<()> {
        Mailbox::move_to(&mut self.imap, folder, id, dest)
    }

    fn append(&mut self, folder: &str, message: &[u8]) -> Result<()> {
        Mailbox::append(&mut self.imap, folder, message)
    }

    fn send(&mut self, message: &[u8]) -> Result<bool> {
        Mailbox::send(&mut self.imap, message)
    }

    fn create_folder(&mut self, parent: Option<&str>, name: &str) -> Result<()> {
        Mailbox::create_folder(&mut self.imap, parent, name)
    }

    fn rename_folder(&mut self, folder: &str, name: &str) -> Result<()> {
        Mailbox::rename_folder(&mut self.imap, folder, name)
    }

    fn delete_folder(&mut self, folder: &str) -> Result<()> {
        Mailbox::delete_folder(&mut self.imap, folder)
    }

    fn delete_message(&mut self, folder: &str, id: &str) -> Result<()> {
        Mailbox::delete_message(&mut self.imap, folder, id)
    }

    fn empty_folder(&mut self, folder: &str) -> Result<()> {
        Mailbox::empty_folder(&mut self.imap, folder)
    }

    fn set_categories(&mut self, folder: &str, id: &str, categories: &[String], removed: &[String]) -> Result<()> {
        Mailbox::set_categories(&mut self.imap, folder, id, categories, removed)
    }

    fn items(&mut self) -> Option<&mut dyn ItemStore> {
        Some(&mut self.items)
    }

    fn logout(self: Box<Self>) {
        Mailbox::logout(Box::new(self.imap));
    }
}

/// A calendar or address book on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    /// Absolute URL, ending in `/`.
    pub url: String,
    pub name: String,
    /// For a calendar: whether it takes VEVENTs, and VTODOs.
    pub events: bool,
    pub todos: bool,
}

/// Status, lower-cased headers and body of one HTTP reply.
type Reply = (u16, Vec<(String, String)>, String);

/// A resource as last read, so a change can be written back into it.
#[derive(Clone, Debug)]
struct Resource {
    etag: Option<String>,
    text: String,
}

/// The CalDAV and CardDAV side of an account.
pub struct DavClient {
    agent: ureq::Agent,
    username: String,
    password: Secret,
    calendars: Vec<Collection>,
    addressbooks: Vec<Collection>,
    seen: HashMap<String, Resource>,
    next_uid: u64,
}

/// `url` with a trailing slash, as a collection URL is written.
fn with_slash(url: &str) -> String {
    let url = url.trim();
    if url.ends_with('/') { url.to_string() } else { format!("{url}/") }
}

/// `scheme://host[:port]` of `url`.
fn origin(url: &str) -> String {
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    let end = url.get(after_scheme..).and_then(|rest| rest.find('/')).map(|i| after_scheme + i).unwrap_or(url.len());
    url.get(..end).unwrap_or(url).to_string()
}

/// The host of `url`, for error messages.
fn host_of(url: &str) -> String {
    origin(url).split("://").nth(1).unwrap_or(url).to_string()
}

/// `href` as the server wrote it, made absolute against `base`.
fn resolve(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        format!("{}{href}", origin(base))
    } else {
        let dir = base.rfind('/').map(|i| base.get(..=i).unwrap_or(base)).unwrap_or(base);
        format!("{dir}{href}")
    }
}

fn is(n: Node, ns: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)
}

fn child<'a>(n: Node<'a, 'a>, ns: &str, name: &str) -> Option<Node<'a, 'a>> {
    n.children().find(|c| is(*c, ns, name))
}

fn descendant<'a>(n: Node<'a, 'a>, ns: &str, name: &str) -> Option<Node<'a, 'a>> {
    n.descendants().find(|c| is(*c, ns, name))
}

fn text(n: Option<Node>) -> String {
    n.and_then(|n| n.text()).map(|t| t.trim().to_string()).unwrap_or_default()
}

/// Each `D:response` of a multistatus: its href (as written) and the `D:prop` of its 200 propstat.
fn responses<'a>(doc: &'a Document<'a>) -> Vec<(String, Option<Node<'a, 'a>>)> {
    let mut out = Vec::new();
    for r in doc.root_element().children().filter(|c| is(*c, DAV, "response")) {
        let href = text(child(r, DAV, "href"));
        if href.is_empty() {
            continue;
        }
        let prop = r
            .children()
            .filter(|c| is(*c, DAV, "propstat"))
            .find(|p| text(child(*p, DAV, "status")).contains(" 200 "))
            .and_then(|p| child(p, DAV, "prop"));
        out.push((href, prop));
    }
    out
}

/// What `id` points at: `(kind, url, recurrence)`.
fn parse_id(id: &str) -> Result<(char, String, Option<String>)> {
    let (kind, rest) = id.split_once(':').ok_or_else(|| MailboxError::BadName(id.chars().take(40).collect()))?;
    let kind = match kind {
        "e" => 'e',
        "c" => 'c',
        "t" => 't',
        _ => return Err(MailboxError::BadName(id.chars().take(40).collect())),
    };
    let (url, recurrence) = match rest.split_once('#') {
        Some((u, r)) => (u.to_string(), Some(r.to_string())),
        None => (rest.to_string(), None),
    };
    if !url.starts_with("https://") || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(MailboxError::BadName(id.chars().take(40).collect()));
    }
    Ok((kind, url, recurrence))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn ical_time(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0).map(|d| d.format("%Y%m%dT%H%M%SZ").to_string()).unwrap_or_else(|| "19700101T000000Z".into())
}

const RECURRING: &str = "Herhalende afspraken kun je nog niet bewerken in Mailcraft; doe dat in de agenda van je aanbieder.";

impl DavClient {
    /// Finds the calendars and address books behind `dav` for `username`.
    pub fn connect(dav: &Dav, username: &str, password: Secret) -> Result<Self> {
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(Duration::from_secs(60)))
                .http_status_as_error(false)
                .max_redirects(0)
                .user_agent("Mailcraft")
                .build(),
        );
        let mut client = DavClient {
            agent,
            username: username.to_string(),
            password,
            calendars: Vec::new(),
            addressbooks: Vec::new(),
            seen: HashMap::new(),
            next_uid: 0,
        };
        let calendar_base = with_slash(&dav.calendar);
        let principal = client.principal(&calendar_base, "caldav")?;
        let home = client.home(&principal, CALDAV, "calendar-home-set")?;
        client.calendars = client.collections(&home, CALDAV, "calendar")?;
        if client.calendars.iter().all(|c| !c.events) {
            return Err(MailboxError::Protocol(format!("Geen agenda gevonden op {}.", host_of(&calendar_base))));
        }
        let contacts_base = dav.contacts.as_deref().map(with_slash);
        let addressbooks = client
            .principal(contacts_base.as_deref().unwrap_or(&calendar_base), "carddav")
            .and_then(|p| client.home(&p, CARDDAV, "addressbook-home-set"))
            .and_then(|home| client.collections(&home, CARDDAV, "addressbook"));
        client.addressbooks = match (addressbooks, &contacts_base) {
            (Ok(books), _) => books,
            (Err(e), Some(base)) => return Err(MailboxError::Protocol(format!("Adresboek op {}: {e}", host_of(base)))),
            (Err(e), None) => {
                log::info!("no address book next to the calendar: {e}");
                Vec::new()
            }
        };
        Ok(client)
    }

    /// The calendars found, for tests and the log.
    pub fn calendars(&self) -> &[Collection] {
        &self.calendars
    }

    pub fn addressbooks(&self) -> &[Collection] {
        &self.addressbooks
    }

    /// One request, redirects followed by hand (same method, same body). Returns the status,
    /// the headers and the body text.
    fn request(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<(&str, &[u8])>) -> Result<Reply> {
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.username, self.password.expose())));
        let mut url = url.to_string();
        for _ in 0..=MAX_REDIRECTS {
            if !url.starts_with("https://") {
                return Err(MailboxError::Protocol("de DAV-server moet via https bereikbaar zijn".into()));
            }
            let mut builder = ureq::http::Request::builder().method(method).uri(&url).header("Authorization", &auth);
            for (k, v) in headers {
                builder = builder.header(*k, *v);
            }
            let request = match body {
                Some((content_type, bytes)) => builder.header("Content-Type", content_type).body(bytes.to_vec()),
                None => builder.body(Vec::new()),
            }
            .map_err(|e| MailboxError::Protocol(e.to_string()))?;
            let host = host_of(&url);
            let mut response = self.agent.run(request).map_err(|e| MailboxError::Connect { host: host.clone(), reason: e.to_string() })?;
            let status = response.status().as_u16();
            let headers: Vec<(String, String)> =
                response.headers().iter().map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string())).collect();
            if matches!(status, 301 | 302 | 307 | 308)
                && let Some((_, location)) = headers.iter().find(|(k, _)| k == "location")
            {
                url = resolve(&url, location);
                continue;
            }
            let text = response.body_mut().with_config().limit(MAX_REPLY).read_to_string().map_err(|e| MailboxError::Protocol(e.to_string()))?;
            return Ok((status, headers, text));
        }
        Err(MailboxError::Protocol("te veel omleidingen".into()))
    }

    /// A PROPFIND or REPORT whose answer is a multistatus, as XML text.
    fn multistatus(&self, method: &str, url: &str, depth: &str, body: &str) -> Result<String> {
        let (status, _, text) = self.request(method, url, &[("Depth", depth)], Some(("application/xml; charset=utf-8", body.as_bytes())))?;
        match status {
            207 => Ok(text),
            401 => Err(MailboxError::Login),
            s => Err(MailboxError::Protocol(format!("HTTP {s} op {}", clean_text(&host_of(url))))),
        }
    }

    /// Where `/.well-known/<kind>` on the server points, else `base` itself.
    fn well_known(&self, base: &str, kind: &str) -> String {
        let url = format!("{}/.well-known/{kind}", origin(base));
        // Redirects are followed inside `request`; what comes back tells where we ended up
        // only through the principal lookup, so a plain probe decides whether the path exists.
        match self.request("PROPFIND", &url, &[("Depth", "0")], Some(("application/xml; charset=utf-8", PRINCIPAL_QUERY.as_bytes()))) {
            Ok((207, _, _)) => url,
            _ => base.to_string(),
        }
    }

    /// The current user's principal URL, found from `base` (through its well-known path).
    fn principal(&self, base: &str, kind: &str) -> Result<String> {
        let start = self.well_known(base, kind);
        let xml = self.multistatus("PROPFIND", &start, "0", PRINCIPAL_QUERY).or_else(|_| self.multistatus("PROPFIND", base, "0", PRINCIPAL_QUERY))?;
        let doc = Document::parse(&xml).map_err(|e| MailboxError::Protocol(e.to_string()))?;
        let href = responses(&doc)
            .into_iter()
            .find_map(|(_, prop)| {
                prop.and_then(|p| descendant(p, DAV, "current-user-principal")).and_then(|c| child(c, DAV, "href")).map(|h| text(Some(h)))
            })
            .filter(|h| !h.is_empty())
            .ok_or_else(|| MailboxError::Protocol(format!("{} noemt geen gebruikersprincipal", host_of(base))))?;
        Ok(resolve(&start, &href))
    }

    /// The home collection named by `prop` (calendar-home-set, addressbook-home-set).
    fn home(&self, principal: &str, ns: &str, prop: &str) -> Result<String> {
        let prefix = if ns == CALDAV { "C" } else { "CR" };
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:" xmlns:{prefix}="{ns}"><D:prop><{prefix}:{prop}/></D:prop></D:propfind>"#
        );
        let xml = self.multistatus("PROPFIND", principal, "0", &body)?;
        let doc = Document::parse(&xml).map_err(|e| MailboxError::Protocol(e.to_string()))?;
        let href = responses(&doc)
            .into_iter()
            .find_map(|(_, p)| p.and_then(|p| descendant(p, ns, prop)).and_then(|h| child(h, DAV, "href")).map(|h| text(Some(h))))
            .filter(|h| !h.is_empty())
            .ok_or_else(|| MailboxError::Protocol(format!("{} noemt geen {prop}", host_of(principal))))?;
        Ok(with_slash(&resolve(principal, &href)))
    }

    /// The collections of `kind` (`calendar`, `addressbook`) in `home`.
    fn collections(&self, home: &str, ns: &str, kind: &str) -> Result<Vec<Collection>> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><D:displayname/><D:resourcetype/><C:supported-calendar-component-set/></D:prop></D:propfind>"#;
        let xml = self.multistatus("PROPFIND", home, "1", body)?;
        let doc = Document::parse(&xml).map_err(|e| MailboxError::Protocol(e.to_string()))?;
        Ok(parse_collections(&doc, home, ns, kind))
    }

    fn basic_status(status: u16, url: &str) -> Result<()> {
        match status {
            200..=299 => Ok(()),
            401 => Err(MailboxError::Login),
            404 | 410 => Err(MailboxError::Protocol("Dit item bestaat niet meer op de server.".into())),
            412 => Err(MailboxError::Protocol("Dit item is op de server gewijzigd; vernieuw en probeer het opnieuw.".into())),
            s => Err(MailboxError::Protocol(format!("HTTP {s} op {}", clean_text(&host_of(url))))),
        }
    }

    /// The resources a REPORT returns, remembered for later changes: `(url, components)`.
    fn report(&mut self, collection: &str, body: &str, data_ns: &str, data_name: &str) -> Result<Vec<(String, Vec<Component>)>> {
        let xml = self.multistatus("REPORT", collection, "1", body)?;
        let doc = Document::parse(&xml).map_err(|e| MailboxError::Protocol(e.to_string()))?;
        let mut out = Vec::new();
        for (href, prop) in responses(&doc) {
            let Some(prop) = prop else { continue };
            let data = text_of(child(prop, data_ns, data_name));
            if data.trim().is_empty() {
                continue;
            }
            let url = resolve(collection, &href);
            let etag = Some(text(child(prop, DAV, "getetag"))).filter(|e| !e.is_empty());
            let components = ical::parse(&data);
            self.seen.insert(url.clone(), Resource { etag, text: data });
            out.push((url, components));
        }
        Ok(out)
    }

    fn events_query(from: i64, to: i64, expand: bool) -> String {
        let (start, end) = (ical_time(from), ical_time(to.max(from + 1)));
        let data = if expand {
            format!(r#"<C:calendar-data><C:expand start="{start}" end="{end}"/></C:calendar-data>"#)
        } else {
            "<C:calendar-data/>".to_string()
        };
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><D:getetag/>{data}</D:prop><C:filter><C:comp-filter name="VCALENDAR"><C:comp-filter name="VEVENT"><C:time-range start="{start}" end="{end}"/></C:comp-filter></C:comp-filter></C:filter></C:calendar-query>"#
        )
    }

    const TODO_QUERY: &'static str = r#"<?xml version="1.0" encoding="utf-8"?><C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><D:getetag/><C:calendar-data/></D:prop><C:filter><C:comp-filter name="VCALENDAR"><C:comp-filter name="VTODO"/></C:comp-filter></C:filter></C:calendar-query>"#;

    /// The hrefs of the cards in an address book.
    fn card_hrefs(&self, book: &str) -> Result<Vec<String>> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:getetag/><D:resourcetype/></D:prop></D:propfind>"#;
        let xml = self.multistatus("PROPFIND", book, "1", body)?;
        let doc = Document::parse(&xml).map_err(|e| MailboxError::Protocol(e.to_string()))?;
        Ok(responses(&doc)
            .into_iter()
            .filter(|(href, prop)| {
                let is_collection = prop.and_then(|p| child(p, DAV, "resourcetype")).is_some_and(|r| child(r, DAV, "collection").is_some());
                !is_collection && resolve(book, href) != *book
            })
            .map(|(href, _)| href)
            .collect())
    }

    /// Writes `component` to `url`; `etag` guards a replacement, `None` means it must be new.
    fn put(&mut self, url: &str, content_type: &str, component: &Component, etag: Option<&str>) -> Result<()> {
        let text = component.serialize();
        let guard: (&str, &str) = match etag {
            Some(e) => ("If-Match", e),
            None => ("If-None-Match", "*"),
        };
        let (status, headers, _) = self.request("PUT", url, &[guard], Some((content_type, text.as_bytes())))?;
        Self::basic_status(status, url)?;
        let new_etag = headers.iter().find(|(k, _)| k == "etag").map(|(_, v)| v.clone());
        match new_etag {
            // Without an ETag in the reply the next change starts by fetching the resource.
            Some(etag) => self.seen.insert(url.to_string(), Resource { etag: Some(etag), text }),
            None => self.seen.remove(url),
        };
        Ok(())
    }

    /// The resource at `url` as last seen, fetched when it is not.
    fn resource(&mut self, url: &str) -> Result<Resource> {
        if let Some(r) = self.seen.get(url) {
            return Ok(r.clone());
        }
        let (status, headers, text) = self.request("GET", url, &[], None)?;
        Self::basic_status(status, url)?;
        let etag = headers.iter().find(|(k, _)| k == "etag").map(|(_, v)| v.clone());
        let r = Resource { etag, text };
        self.seen.insert(url.to_string(), r.clone());
        Ok(r)
    }

    fn uid(&mut self) -> String {
        self.next_uid += 1;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        format!("{nanos:x}-{}@mailcraft", self.next_uid)
    }

    fn first_calendar(&self, todos: bool) -> Result<String> {
        self.calendars
            .iter()
            .find(|c| if todos { c.todos } else { c.events })
            .map(|c| c.url.clone())
            .ok_or_else(|| MailboxError::Protocol(if todos { "Deze server bewaart geen taken.".into() } else { "Geen agenda gevonden.".into() }))
    }

    /// Creates or replaces one calendar object (`VEVENT` or `VTODO`).
    fn save_calendar_object(&mut self, id: &str, kind: &str, prefix: char, write: &dyn Fn(&mut Component)) -> Result<String> {
        if id.is_empty() {
            let collection = self.first_calendar(kind == "VTODO")?;
            let uid = self.uid();
            let mut root = ical::new_calendar(kind, &uid);
            if let Some(inner) = root.find_mut(kind) {
                write(inner);
            }
            let url = format!("{collection}{uid}.ics");
            self.put(&url, "text/calendar; charset=utf-8", &root, None)?;
            return Ok(format!("{prefix}:{url}"));
        }
        let (_, url, recurrence) = parse_id(id)?;
        if recurrence.is_some() {
            return Err(MailboxError::Protocol(RECURRING.into()));
        }
        let resource = self.resource(&url)?;
        let mut roots = ical::parse(&resource.text);
        let Some(root) = roots.first_mut() else { return Err(MailboxError::Protocol("Dit item is niet leesbaar.".into())) };
        let instances = root.all(kind);
        if instances.len() != 1 || instances.iter().any(|c| c.prop("RRULE").is_some() || c.prop("RDATE").is_some()) {
            return Err(MailboxError::Protocol(RECURRING.into()));
        }
        if let Some(inner) = root.find_mut(kind) {
            write(inner);
        }
        self.put(&url, "text/calendar; charset=utf-8", root, resource.etag.as_deref())?;
        Ok(id.to_string())
    }
}

const PRINCIPAL_QUERY: &str =
    r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:current-user-principal/></D:prop></D:propfind>"#;

/// The text of a node, including CDATA pieces.
fn text_of(n: Option<Node>) -> String {
    n.map(|n| n.children().filter(|c| c.is_text()).filter_map(|c| c.text()).collect::<String>()).unwrap_or_default()
}

/// The collections of `kind` in a PROPFIND answer for `home`.
fn parse_collections(doc: &Document, home: &str, ns: &str, kind: &str) -> Vec<Collection> {
    let mut out = Vec::new();
    for (href, prop) in responses(doc) {
        let Some(prop) = prop else { continue };
        let Some(resourcetype) = child(prop, DAV, "resourcetype") else { continue };
        if child(resourcetype, ns, kind).is_none() {
            continue;
        }
        let url = with_slash(&resolve(home, &href));
        let name = clean_text(&text(child(prop, DAV, "displayname")));
        let name = if name.is_empty() { url.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string() } else { name };
        let (events, todos) = match child(prop, CALDAV, "supported-calendar-component-set") {
            Some(set) if set.children().any(|c| is(c, CALDAV, "comp")) => {
                let names: Vec<String> =
                    set.children().filter(|c| is(*c, CALDAV, "comp")).filter_map(|c| c.attribute("name")).map(str::to_uppercase).collect();
                (names.iter().any(|n| n == "VEVENT"), names.iter().any(|n| n == "VTODO"))
            }
            _ => (true, true),
        };
        out.push(Collection { url, name, events, todos });
    }
    out
}

impl ItemStore for DavClient {
    fn appointments(&mut self, from: i64, to: i64) -> Result<Vec<Appointment>> {
        let calendars: Vec<String> = self.calendars.iter().filter(|c| c.events).map(|c| c.url.clone()).collect();
        let mut out = Vec::new();
        for calendar in calendars {
            // Servers that cannot expand recurrences answer the plain query; those show the
            // first occurrence only.
            let resources = match self.report(&calendar, &Self::events_query(from, to, true), CALDAV, "calendar-data") {
                Ok(r) => r,
                Err(MailboxError::Login) => return Err(MailboxError::Login),
                Err(_) => self.report(&calendar, &Self::events_query(from, to, false), CALDAV, "calendar-data")?,
            };
            for (url, roots) in resources {
                for root in &roots {
                    for event in root.all("VEVENT") {
                        let id = match event.prop("RECURRENCE-ID") {
                            Some(r) => format!("e:{url}#{}", r.raw.trim()),
                            None if root.all("VEVENT").len() > 1 => format!("e:{url}#{}", event.prop("DTSTART").map(|p| p.raw.trim()).unwrap_or("")),
                            None => format!("e:{url}"),
                        };
                        if let Some(a) = ical::appointment_of(event, id) {
                            out.push(a);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    fn contacts(&mut self) -> Result<Vec<Contact>> {
        let books: Vec<String> = self.addressbooks.iter().map(|b| b.url.clone()).collect();
        let mut out = Vec::new();
        for book in books {
            let hrefs = self.card_hrefs(&book)?;
            for batch in hrefs.chunks(CARD_BATCH) {
                let list: String = batch.iter().map(|h| format!("<D:href>{}</D:href>", xml_escape(h))).collect();
                let body = format!(
                    r#"<?xml version="1.0" encoding="utf-8"?><CR:addressbook-multiget xmlns:D="DAV:" xmlns:CR="urn:ietf:params:xml:ns:carddav"><D:prop><D:getetag/><CR:address-data/></D:prop>{list}</CR:addressbook-multiget>"#
                );
                for (url, roots) in self.report(&book, &body, CARDDAV, "address-data")? {
                    if let Some(card) = roots.iter().find(|r| r.name == "VCARD")
                        && let Some(c) = ical::contact_of(card, format!("c:{url}"))
                    {
                        out.push(c);
                    }
                }
            }
        }
        Ok(out)
    }

    fn tasks(&mut self) -> Result<Vec<Task>> {
        let calendars: Vec<String> = self.calendars.iter().filter(|c| c.todos).map(|c| c.url.clone()).collect();
        let mut out = Vec::new();
        for calendar in calendars {
            for (url, roots) in self.report(&calendar, Self::TODO_QUERY, CALDAV, "calendar-data")? {
                if let Some(todo) = roots.iter().find_map(|r| r.find("VTODO"))
                    && let Some(t) = ical::task_of(todo, format!("t:{url}"))
                {
                    out.push(t);
                }
            }
        }
        Ok(out)
    }

    fn save(&mut self, item: &Item) -> Result<String> {
        let id = item.id();
        match item {
            Item::Appointment(a) => self.save_calendar_object(id, "VEVENT", 'e', &|c| ical::write_appointment(c, a)),
            Item::Task(t) => self.save_calendar_object(id, "VTODO", 't', &|c| ical::write_task(c, t)),
            Item::Contact(c) => {
                if id.is_empty() {
                    let book = self
                        .addressbooks
                        .first()
                        .map(|b| b.url.clone())
                        .ok_or_else(|| MailboxError::Protocol("Geen adresboek gevonden op de server.".into()))?;
                    let uid = self.uid();
                    let mut card = ical::new_card(&uid);
                    ical::write_contact(&mut card, c);
                    let url = format!("{book}{uid}.vcf");
                    self.put(&url, "text/vcard; charset=utf-8", &card, None)?;
                    return Ok(format!("c:{url}"));
                }
                let (_, url, _) = parse_id(id)?;
                let resource = self.resource(&url)?;
                let mut roots = ical::parse(&resource.text);
                let Some(card) = roots.iter_mut().find(|r| r.name == "VCARD") else {
                    return Err(MailboxError::Protocol("Dit contact is niet leesbaar.".into()));
                };
                ical::write_contact(card, c);
                let card = card.clone();
                self.put(&url, "text/vcard; charset=utf-8", &card, resource.etag.as_deref())?;
                Ok(id.to_string())
            }
        }
    }

    fn delete(&mut self, id: &str) -> Result<()> {
        let (_, url, recurrence) = parse_id(id)?;
        if recurrence.is_some() {
            return Err(MailboxError::Protocol(RECURRING.into()));
        }
        let (status, _, _) = self.request("DELETE", &url, &[], None)?;
        self.seen.remove(&url);
        match status {
            // Deleting what is already gone is done.
            404 | 410 => Ok(()),
            s => Self::basic_status(s, &url),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_resolve_like_a_browser() {
        assert_eq!(origin("https://dav.example.nl/x/y"), "https://dav.example.nl");
        assert_eq!(origin("https://dav.example.nl:8443"), "https://dav.example.nl:8443");
        assert_eq!(resolve("https://dav.example.nl/a/b/", "/principals/u/"), "https://dav.example.nl/principals/u/");
        assert_eq!(resolve("https://dav.example.nl/a/b/", "c.ics"), "https://dav.example.nl/a/b/c.ics");
        assert_eq!(resolve("https://dav.example.nl/a/b", "https://other/x"), "https://other/x");
        assert_eq!(with_slash("https://x/y"), "https://x/y/");
        assert_eq!(host_of("https://caldav.fastmail.com/dav/"), "caldav.fastmail.com");
    }

    #[test]
    fn ids_name_the_resource_and_mark_recurrences() {
        assert_eq!(parse_id("e:https://x/cal/1.ics").unwrap(), ('e', "https://x/cal/1.ics".into(), None));
        assert_eq!(parse_id("e:https://x/cal/1.ics#20261019T080000Z").unwrap().2, Some("20261019T080000Z".into()));
        assert!(parse_id("x:https://x/1").is_err());
        assert!(parse_id("c:http://x/1").is_err(), "plain http never");
        assert!(parse_id("t:https://x/1 2").is_err());
    }

    #[test]
    fn collections_come_out_of_a_multistatus() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav">
          <d:response><d:href>/dav/calendars/user/a@b.nl/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/dav/calendars/user/a@b.nl/work/</d:href><d:propstat><d:prop><d:displayname>Werk &amp; privé</d:displayname><d:resourcetype><d:collection/><cal:calendar/></d:resourcetype>
            <cal:supported-calendar-component-set><cal:comp name="VEVENT"/></cal:supported-calendar-component-set></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/dav/calendars/user/a@b.nl/tasks/</d:href><d:propstat><d:prop><d:displayname/><d:resourcetype><d:collection/><cal:calendar/></d:resourcetype>
            <cal:supported-calendar-component-set><cal:comp name="VTODO"/></cal:supported-calendar-component-set></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/dav/calendars/user/a@b.nl/old/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><cal:calendar/></d:resourcetype></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
        </d:multistatus>"#;
        let doc = Document::parse(xml).unwrap();
        let found = parse_collections(&doc, "https://caldav.example.nl/dav/calendars/user/a@b.nl/", CALDAV, "calendar");
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(
            (found[0].url.as_str(), found[0].name.as_str(), found[0].events, found[0].todos),
            ("https://caldav.example.nl/dav/calendars/user/a@b.nl/work/", "Werk & privé", true, false)
        );
        assert_eq!((found[1].name.as_str(), found[1].events, found[1].todos), ("tasks", false, true));
        assert!(parse_collections(&doc, "https://x/", CARDDAV, "addressbook").is_empty());
    }

    #[test]
    fn multistatus_responses_carry_href_and_the_ok_prop() {
        let xml = r#"<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/c/1.ics</D:href>
          <D:propstat><D:prop><D:getetag>"abc"</D:getetag><C:calendar-data>BEGIN:VCALENDAR
BEGIN:VEVENT
UID:1
DTSTART:20261019T080000Z
SUMMARY:Lunch &amp; meer
END:VEVENT
END:VCALENDAR
</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        let doc = Document::parse(xml).unwrap();
        let r = responses(&doc);
        assert_eq!(r.len(), 1);
        let prop = r[0].1.unwrap();
        assert_eq!(text(child(prop, DAV, "getetag")), "\"abc\"");
        let data = text_of(child(prop, CALDAV, "calendar-data"));
        let roots = ical::parse(&data);
        let a = ical::appointment_of(roots[0].find("VEVENT").unwrap(), "e:x".into()).unwrap();
        assert_eq!(a.title, "Lunch & meer");
        assert!(DavClient::events_query(0, 3600, true).contains(r#"<C:expand start="19700101T000000Z" end="19700101T010000Z"/>"#));
        assert!(!DavClient::events_query(0, 3600, false).contains("expand"));
    }
}
