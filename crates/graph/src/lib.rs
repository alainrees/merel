//! Microsoft 365 through Microsoft Graph: mail folders, messages, sending, and the calendar,
//! contacts and To Do tasks, all with one OAuth token for the one resource
//! `graph.microsoft.com`.
//!
//! Why Graph for everything: Exchange Online stops answering EWS in October 2026, and IMAP and
//! Graph are separate OAuth resources that one browser sign-in cannot cover, so a Microsoft 365
//! account that wants its calendar along has to read its mail through Graph too.
//!
//! Every call is HTTPS with the platform certificate verifier; ids go into URLs
//! percent-encoded; replies are read with a size cap; a 401 is reported as a refused login,
//! which makes the worker fetch a fresh token and retry once. Store ids carry their kind
//! (`event:`, `contact:`, `task:`) since the APIs are three.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Utc};
use mailcraft_mailbox::{Credential, Flag, Item, ItemStore, Mailbox, MailboxError, RemoteFolder, RemoteHeader, Result, kind_from_name};
use mailcraft_mime::ParsedHeader;
use mailcraft_model::{
    Address, Appointment, Attendee, Contact, FolderKind, Frequency, Importance, Meeting, Priority, Recurrence, Response, Server, ShowAs, Task,
    clean_text,
};
use mailcraft_secrets::Secret;
use serde_json::{Value, json};

/// The largest reply accepted: a whole message as MIME, with room.
const MAX_REPLY: u64 = 32 * 1024 * 1024;
/// Items per page, and pages followed per listing.
const PAGE: u32 = 1000;
const MAX_PAGES: usize = 5;
/// The well-known mail folders, with the kind each one is.
const WELL_KNOWN: [(&str, FolderKind); 6] = [
    ("inbox", FolderKind::Inbox),
    ("drafts", FolderKind::Drafts),
    ("sentitems", FolderKind::Sent),
    ("deleteditems", FolderKind::Deleted),
    ("junkemail", FolderKind::Junk),
    ("archive", FolderKind::Archive),
];
/// Times in replies come in UTC and bodies as text, whatever the mailbox is set to.
const PREFER: &str = "outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"";

pub struct GraphClient {
    agent: ureq::Agent,
    /// `https://graph.microsoft.com/v1.0`, or a national cloud's.
    base: String,
    host: String,
    token: Secret,
    /// Ids of the well-known folders, learned at connect.
    known: Vec<(String, FolderKind)>,
    /// The computer's time zone by its IANA name (Graph takes those), for all-day
    /// appointments and due days.
    time_zone: String,
    /// The id of the default To Do list, once looked up.
    task_list: Option<String>,
}

/// `s` percent-encoded for a URL path segment or query value.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `2026-10-18T07:00:00Z` or `2026-10-18T07:00:00.0000000` (Graph's UTC shapes) as seconds.
fn parse_utc(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    let s = s.split('.').next()?;
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok().map(|d| d.and_utc().timestamp())
}

/// A `dateTimeTimeZone` as seconds; only UTC is understood, which is what [`PREFER`] asks for.
fn moment(v: &Value) -> Option<i64> {
    let dt = v.get("dateTime").and_then(Value::as_str)?;
    if v.get("timeZone").and_then(Value::as_str).is_some_and(|z| z != "UTC") {
        log::debug!("a Graph time came back in {:?}, read as UTC", v["timeZone"]);
    }
    parse_utc(dt)
}

fn utc(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0).map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_else(|| utc(0))
}

/// `ts` as local wall-clock time without an offset, for a `dateTimeTimeZone` naming the zone.
fn local(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).single().map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_else(|| utc(ts))
}

/// A [`Recurrence`] as Graph's `patternedRecurrence`: the pattern (how often) and the range
/// (from the start's day; for ever, until a day, or so many times).
fn recurrence_json(r: &Recurrence, start: i64, zone: &str) -> Value {
    const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    let first = Local.timestamp_opt(start, 0).single().unwrap_or_default();
    let pattern = match r.frequency {
        Frequency::Daily => json!({ "type": "daily", "interval": r.interval }),
        Frequency::Weekly => {
            let days: Vec<&str> = if r.weekdays.is_empty() {
                vec![DAYS[first.weekday().num_days_from_monday() as usize]]
            } else {
                r.weekdays.iter().filter_map(|d| DAYS.get(usize::from(*d))).copied().collect()
            };
            json!({ "type": "weekly", "interval": r.interval, "daysOfWeek": days, "firstDayOfWeek": "monday" })
        }
        Frequency::Monthly => json!({ "type": "absoluteMonthly", "interval": r.interval, "dayOfMonth": first.day() }),
        Frequency::Yearly => json!({ "type": "absoluteYearly", "interval": r.interval, "dayOfMonth": first.day(), "month": first.month() }),
    };
    let start_date = first.format("%Y-%m-%d").to_string();
    let range = match (r.until, r.count) {
        (Some(u), _) => {
            let end = Local.timestamp_opt(u, 0).single().unwrap_or_default().format("%Y-%m-%d").to_string();
            json!({ "type": "endDate", "startDate": start_date, "endDate": end, "recurrenceTimeZone": zone })
        }
        (None, Some(c)) => json!({ "type": "numbered", "startDate": start_date, "numberOfOccurrences": c, "recurrenceTimeZone": zone }),
        (None, None) => json!({ "type": "noEnd", "startDate": start_date, "recurrenceTimeZone": zone }),
    };
    json!({ "pattern": pattern, "range": range })
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn id_of(v: &Value) -> Option<String> {
    v.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_string)
}

fn title_of(v: &Value, key: &str) -> String {
    let t = clean_text(str_of(v, key));
    if t.is_empty() { "(Zonder titel)".into() } else { t }
}

fn body_text(v: &Value) -> String {
    v.get("body").map(|b| str_of(b, "content")).unwrap_or("").to_string()
}

fn text_body(content: &str) -> Value {
    json!({ "contentType": "text", "content": content })
}

fn address_of(v: &Value) -> Option<Address> {
    let e = v.get("emailAddress")?;
    let address = str_of(e, "address");
    let name = Some(str_of(e, "name")).filter(|n| !n.is_empty() && *n != address);
    Address::new(name, address).ok()
}

fn header_of(m: &Value) -> Option<RemoteHeader> {
    let id = id_of(m)?;
    let date = m.get("receivedDateTime").and_then(Value::as_str).and_then(parse_utc).unwrap_or(0);
    let to = m.get("toRecipients").and_then(Value::as_array).map(|l| l.iter().filter_map(address_of).collect()).unwrap_or_default();
    let message_id = Some(clean_text(str_of(m, "internetMessageId"))).filter(|s| !s.is_empty());
    Some(RemoteHeader {
        id,
        seen: m.get("isRead").and_then(Value::as_bool).unwrap_or(false),
        flagged: m.get("flag").map(|f| str_of(f, "flagStatus")) == Some("flagged"),
        // Graph does not say whether a message was answered.
        answered: false,
        size: 0,
        has_attachments: m.get("hasAttachments").and_then(Value::as_bool).unwrap_or(false),
        categories: m
            .get("categories")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).map(clean_text).collect())
            .unwrap_or_default(),
        header: ParsedHeader {
            from: m.get("from").and_then(address_of),
            to,
            subject: clean_text(str_of(m, "subject")),
            date: Some(date),
            message_id,
            importance: match str_of(m, "importance") {
                "high" => Importance::High,
                "low" => Importance::Low,
                _ => Importance::Normal,
            },
        },
        date,
    })
}

fn appointment_of(e: &Value) -> Option<Appointment> {
    if e.get("isCancelled").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let start = e.get("start").and_then(moment)?;
    let end = e.get("end").and_then(moment)?;
    Appointment {
        id: format!("event:{}", id_of(e)?),
        calendar_id: String::new(),
        title: title_of(e, "subject"),
        start,
        end: end.max(start + 1),
        all_day: e.get("isAllDay").and_then(Value::as_bool).unwrap_or(false),
        location: e.get("location").map(|l| str_of(l, "displayName")).unwrap_or("").to_string(),
        notes: body_text(e),
        notes_rich: None,
        recurrence: None,
        reminder: (e.get("isReminderOn").and_then(Value::as_bool) == Some(true))
            .then(|| e.get("reminderMinutesBeforeStart").and_then(Value::as_u64).and_then(|m| u32::try_from(m).ok()))
            .flatten(),
        private: matches!(e.get("sensitivity").and_then(Value::as_str), Some("private" | "confidential")),
        show_as: match e.get("showAs").and_then(Value::as_str) {
            Some("free" | "workingElsewhere") => ShowAs::Free,
            Some("tentative") => ShowAs::Tentative,
            Some("oof") => ShowAs::Oof,
            _ => ShowAs::Busy,
        },
        categories: e
            .get("categories")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
        meeting: meeting_of(e),
    }
    .validated()
    .ok()
}

/// The organiser and attendees with their answers, as Microsoft 365 tracks them for a meeting
/// it sent; `None` without attendees.
fn meeting_of(e: &Value) -> Option<Meeting> {
    let attendees: Vec<Attendee> = e
        .get("attendees")
        .and_then(Value::as_array)?
        .iter()
        .filter(|a| a.get("type").and_then(Value::as_str) != Some("resource"))
        .map(|a| {
            let who = a.get("emailAddress").unwrap_or(&Value::Null);
            Attendee {
                email: str_of(who, "address").to_string(),
                name: str_of(who, "name").to_string(),
                optional: a.get("type").and_then(Value::as_str) == Some("optional"),
                response: match a.get("status").and_then(|s| s.get("response")).and_then(Value::as_str) {
                    Some("accepted") => Response::Accepted,
                    Some("tentativelyAccepted") => Response::Tentative,
                    Some("declined") => Response::Declined,
                    _ => Response::None,
                },
                replied: None,
                proposed: None,
            }
        })
        .collect();
    if attendees.is_empty() {
        return None;
    }
    let organizer = e.get("organizer").and_then(|o| o.get("emailAddress")).unwrap_or(&Value::Null);
    Some(Meeting {
        uid: str_of(e, "iCalUId").to_string(),
        organizer: str_of(organizer, "address").to_string(),
        organizer_name: str_of(organizer, "name").to_string(),
        sequence: 0,
        attendees,
    })
}

fn contact_of(c: &Value) -> Option<Contact> {
    let strings =
        |key: &str| c.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
    let mut phones = strings("businessPhones").unwrap_or_default();
    if let Some(m) = c.get("mobilePhone").and_then(Value::as_str) {
        phones.push(m.to_string());
    }
    phones.extend(strings("homePhones").unwrap_or_default());
    let emails = c
        .get("emailAddresses")
        .and_then(Value::as_array)
        .map(|l| l.iter().map(|e| str_of(e, "address").to_string()).filter(|a| Address::new(None, a).is_ok()).collect())
        .unwrap_or_default();
    let name = str_of(c, "displayName");
    let name = if name.is_empty() { format!("{} {}", str_of(c, "givenName"), str_of(c, "surname")) } else { name.to_string() };
    Contact {
        id: format!("contact:{}", id_of(c)?),
        name,
        emails,
        phones,
        company: str_of(c, "companyName").to_string(),
        notes: str_of(c, "personalNotes").to_string(),
        account_id: None,
        notes_rich: None,
    }
    .validated()
    .ok()
}

fn task_of(t: &Value) -> Option<Task> {
    Task {
        id: format!("task:{}", id_of(t)?),
        title: title_of(t, "title"),
        notes: body_text(t),
        due: t.get("dueDateTime").and_then(moment),
        priority: match str_of(t, "importance") {
            "low" => Priority::Low,
            "high" => Priority::High,
            _ => Priority::Normal,
        },
        done: str_of(t, "status") == "completed",
        completed: t.get("completedDateTime").and_then(moment),
        account_id: None,
        repeat: None,
        notes_rich: None,
    }
    .validated()
    .ok()
}

impl GraphClient {
    /// The event as Graph takes it; `zone` is the client's time zone, for all-day ones.
    fn event_json(a: &Appointment, zone: &str) -> Value {
        // A series repeats in the computer's zone, so its times go as wall-clock time there.
        let zoned = a.all_day || a.recurrence.is_some();
        let time = |ts| if zoned { json!({ "dateTime": local(ts), "timeZone": zone }) } else { json!({ "dateTime": utc(ts), "timeZone": "UTC" }) };
        let mut event = json!({
            "subject": a.title,
            "body": text_body(&a.notes),
            "start": time(a.start),
            "end": time(a.end),
            "isAllDay": a.all_day,
            "location": { "displayName": a.location },
            "isReminderOn": a.reminder.is_some(),
            "sensitivity": if a.private { "private" } else { "normal" },
            "showAs": match a.show_as {
                ShowAs::Free => "free",
                ShowAs::Tentative => "tentative",
                ShowAs::Busy => "busy",
                ShowAs::Oof => "oof",
            },
            "categories": a.categories,
        });
        if let Some(m) = a.reminder {
            event["reminderMinutesBeforeStart"] = json!(m);
        }
        if let Some(r) = &a.recurrence {
            event["recurrence"] = recurrence_json(r, a.start, zone);
        }
        event
    }

    /// The contact as Graph takes it: the first phone is the business one, the second the
    /// mobile, the rest home phones. Empty fields are sent, so an update clears them.
    fn contact_json(c: &Contact) -> Value {
        let (given, surname) = c.name.split_once(' ').unwrap_or((c.name.as_str(), ""));
        json!({
            "givenName": given,
            "surname": surname.trim(),
            "companyName": c.company,
            "personalNotes": c.notes,
            "emailAddresses": c.emails.iter().map(|e| json!({ "address": e, "name": c.name })).collect::<Vec<_>>(),
            "businessPhones": c.phones.first().into_iter().collect::<Vec<_>>(),
            "mobilePhone": c.phones.get(1),
            "homePhones": c.phones.iter().skip(2).collect::<Vec<_>>(),
        })
    }

    fn task_json(t: &Task, zone: &str) -> Value {
        let importance = match t.priority {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
        };
        json!({
            "title": t.title,
            "body": text_body(&t.notes),
            "importance": importance,
            "status": if t.done { "completed" } else { "notStarted" },
            "dueDateTime": t.due.map(|d| json!({ "dateTime": local(d), "timeZone": zone })),
        })
    }

    /// Builds the client, checks the token with one small request and learns the well-known
    /// folders. `server.host` names the Graph endpoint, normally `graph.microsoft.com`.
    pub fn connect(server: &Server, credential: &Credential) -> Result<Self> {
        let host = server.host.trim().to_ascii_lowercase();
        if host.is_empty() || host.chars().any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))) {
            return Err(MailboxError::BadName(host));
        }
        let token = match credential {
            Credential::OAuth2(t) => t.clone(),
            Credential::Password(_) => return Err(MailboxError::Protocol("Microsoft Graph needs a browser sign-in, not a password".into())),
        };
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .tls_config(tls())
                .timeout_global(Some(Duration::from_secs(60)))
                .http_status_as_error(false)
                .user_agent("Mailcraft")
                .build(),
        );
        let time_zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
        let mut client = GraphClient { agent, base: format!("https://{host}/v1.0"), host, token, known: Vec::new(), time_zone, task_list: None };
        client.known = client.well_known()?;
        Ok(client)
    }

    /// A client that never connects, for tests of what it builds and reads.
    #[cfg(test)]
    fn offline() -> Self {
        GraphClient {
            agent: ureq::Agent::new_with_config(ureq::config::Config::builder().tls_config(tls()).build()),
            base: "https://graph.microsoft.com/v1.0".into(),
            host: "graph.microsoft.com".into(),
            token: Secret::new(String::new()),
            known: Vec::new(),
            time_zone: "Europe/Amsterdam".into(),
            task_list: None,
        }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("https://") { path.to_string() } else { format!("{}{path}", self.base) }
    }

    /// One request. `body` is JSON unless `content_type` says otherwise. Returns the status
    /// and the raw reply.
    fn request(&self, method: &str, path: &str, content_type: Option<&str>, body: &[u8]) -> Result<(u16, Vec<u8>)> {
        let url = self.url(path);
        let connect = |e: ureq::Error| MailboxError::Connect { host: self.host.clone(), reason: e.to_string() };
        let auth = format!("Bearer {}", self.token.expose());
        let mut response = match method {
            "GET" => self.agent.get(&url).header("Authorization", &auth).header("Prefer", PREFER).call().map_err(connect)?,
            "DELETE" => self.agent.delete(&url).header("Authorization", &auth).call().map_err(connect)?,
            _ => {
                let req = if method == "PATCH" { self.agent.patch(&url) } else { self.agent.post(&url) };
                req.header("Authorization", &auth)
                    .header("Prefer", PREFER)
                    .header("Content-Type", content_type.unwrap_or("application/json"))
                    .send(body)
                    .map_err(connect)?
            }
        };
        let status = response.status().as_u16();
        let bytes = response.body_mut().with_config().limit(MAX_REPLY).read_to_vec().map_err(|e| MailboxError::Protocol(e.to_string()))?;
        Ok((status, bytes))
    }

    /// A JSON call; `Ok(Null)` for an empty reply.
    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let payload = body.map(Value::to_string).unwrap_or_default();
        let (status, bytes) = self.request(method, path, None, payload.as_bytes())?;
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        match status {
            200..=299 => Ok(value),
            401 => Err(MailboxError::Login),
            404 if method == "DELETE" => Ok(Value::Null),
            _ => {
                let message = value.get("error").map(|e| str_of(e, "message")).filter(|m| !m.is_empty()).map(clean_text);
                Err(MailboxError::Protocol(message.unwrap_or_else(|| format!("HTTP {status}"))))
            }
        }
    }

    /// Every item of a paged listing, following `@odata.nextLink` up to `MAX_PAGES` pages.
    fn list(&self, path: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut next = Some(path.to_string());
        for _ in 0..MAX_PAGES {
            let Some(url) = next.take() else { break };
            let page = self.call("GET", &url, None)?;
            if let Some(items) = page.get("value").and_then(Value::as_array) {
                out.extend(items.iter().cloned());
            }
            next = page.get("@odata.nextLink").and_then(Value::as_str).filter(|u| u.starts_with(&self.base)).map(str::to_string);
        }
        Ok(out)
    }

    /// The ids of the well-known folders; a missing one (no archive yet) is skipped.
    fn well_known(&self) -> Result<Vec<(String, FolderKind)>> {
        let mut out = Vec::new();
        for (name, kind) in WELL_KNOWN {
            match self.call("GET", &format!("/me/mailFolders/{name}?$select=id"), None) {
                Ok(v) => {
                    if let Some(id) = id_of(&v) {
                        out.push((id, kind));
                    }
                }
                Err(e @ (MailboxError::Login | MailboxError::Connect { .. })) => return Err(e),
                Err(e) => log::debug!("{}: no {name} folder: {e}", self.host),
            }
        }
        if !out.iter().any(|(_, k)| *k == FolderKind::Inbox) {
            return Err(MailboxError::Protocol("the mailbox has no inbox".into()));
        }
        Ok(out)
    }

    fn kind_of(&self, id: &str, name: &str) -> FolderKind {
        self.known.iter().find(|(k, _)| k == id).map(|(_, kind)| *kind).unwrap_or_else(|| match kind_from_name(name) {
            FolderKind::Archive => FolderKind::Archive,
            _ => FolderKind::Other,
        })
    }

    /// Posts raw MIME to `path` (`/me/sendMail`, or a folder's messages).
    fn post_mime(&self, path: &str, message: &[u8]) -> Result<()> {
        let (status, bytes) = self.request("POST", path, Some("text/plain"), BASE64.encode(message).as_bytes())?;
        match status {
            200..=299 => Ok(()),
            401 => Err(MailboxError::Login),
            _ => {
                let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                let message = value.get("error").map(|e| str_of(e, "message")).filter(|m| !m.is_empty()).map(clean_text);
                Err(MailboxError::Protocol(message.unwrap_or_else(|| format!("HTTP {status}"))))
            }
        }
    }

    /// The default To Do list's id, looked up once.
    fn task_list(&mut self) -> Result<String> {
        if let Some(id) = &self.task_list {
            return Ok(id.clone());
        }
        let lists = self.list("/me/todo/lists?$top=100")?;
        let default = lists.iter().find(|l| str_of(l, "wellknownListName") == "defaultList").or_else(|| lists.first());
        let id = default.and_then(id_of).ok_or_else(|| MailboxError::Protocol("the account has no To Do list".into()))?;
        self.task_list = Some(id.clone());
        Ok(id)
    }

    /// Where an item's id leads.
    fn path_of(&mut self, id: &str) -> Result<String> {
        if let Some(event) = id.strip_prefix("event:") {
            Ok(format!("/me/events/{}", encode(event)))
        } else if let Some(contact) = id.strip_prefix("contact:") {
            Ok(format!("/me/contacts/{}", encode(contact)))
        } else if let Some(task) = id.strip_prefix("task:") {
            Ok(format!("/me/todo/lists/{}/tasks/{}", encode(&self.task_list()?), encode(task)))
        } else {
            Err(MailboxError::BadName(id.chars().take(40).collect()))
        }
    }

    fn created(reply: &Value, prefix: &str) -> Result<String> {
        id_of(reply).map(|id| format!("{prefix}{id}")).ok_or_else(|| MailboxError::Protocol("Graph did not return the new item's id".into()))
    }
}

/// The fields a listing asks for per message.
const MESSAGE_FIELDS: &str = "id,subject,from,toRecipients,receivedDateTime,isRead,flag,hasAttachments,internetMessageId,categories,importance";

/// The request for the messages of `folder` that Graph's `$search` finds for `query`; `None`
/// for an empty query. Graph takes the query as a quoted string (quotes and backslashes
/// escaped) and allows no `$orderby` or `$skip` next to it; `$top` is at most 250 there.
fn search_path(folder: &str, query: &str, limit: u32) -> Option<String> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let quoted = format!("\"{}\"", query.replace('\\', "\\\\").replace('"', "\\\""));
    Some(format!("/me/mailFolders/{}/messages?$search={}&$top={}&$select={MESSAGE_FIELDS}", encode(folder), encode(&quoted), limit.clamp(1, 250)))
}

impl Mailbox for GraphClient {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        let select = "$select=id,displayName,childFolderCount";
        let top: Vec<Value> = self.list(&format!("/me/mailFolders?$top=250&{select}"))?;
        let mut out = Vec::new();
        for f in &top {
            let Some(id) = id_of(f) else { continue };
            let name = clean_text(str_of(f, "displayName"));
            // One level of subfolders, named after their parent like IMAP paths.
            if f.get("childFolderCount").and_then(Value::as_u64).unwrap_or(0) > 0 {
                for c in self.list(&format!("/me/mailFolders/{}/childFolders?$top=250&{select}", encode(&id)))? {
                    let Some(cid) = id_of(&c) else { continue };
                    let cname = clean_text(str_of(&c, "displayName"));
                    out.push(RemoteFolder { path: cid.clone(), kind: self.kind_of(&cid, &cname), name: format!("{name}/{cname}"), selectable: true });
                }
            }
            out.push(RemoteFolder { path: id.clone(), kind: self.kind_of(&id, &name), name, selectable: true });
        }
        if !out.iter().any(|f| f.kind == FolderKind::Inbox) {
            return Err(MailboxError::Protocol("the mailbox has no inbox".into()));
        }
        Ok(out)
    }

    fn headers(&mut self, folder: &str, offset: u32, limit: u32) -> Result<Vec<RemoteHeader>> {
        let path = format!(
            "/me/mailFolders/{}/messages?$top={}&$skip={offset}&$orderby=receivedDateTime%20desc&$select={MESSAGE_FIELDS}",
            encode(folder),
            limit.clamp(1, PAGE)
        );
        let page = self.call("GET", &path, None)?;
        let mut out: Vec<RemoteHeader> =
            page.get("value").and_then(Value::as_array).map(|l| l.iter().filter_map(header_of).collect()).unwrap_or_default();
        out.reverse();
        Ok(out)
    }

    fn search(&mut self, folder: &str, query: &str, limit: u32) -> Result<Vec<RemoteHeader>> {
        let Some(path) = search_path(folder, query, limit) else { return Ok(Vec::new()) };
        let page = self.call("GET", &path, None)?;
        let mut out: Vec<RemoteHeader> =
            page.get("value").and_then(Value::as_array).map(|l| l.iter().filter_map(header_of).collect()).unwrap_or_default();
        // Search results come by relevance; the listing wants them by date, newest last.
        out.sort_by_key(|h| h.date);
        Ok(out)
    }

    fn body(&mut self, _folder: &str, id: &str) -> Result<Vec<u8>> {
        let (status, bytes) = self.request("GET", &format!("/me/messages/{}/$value", encode(id)), None, &[])?;
        match status {
            200..=299 => Ok(bytes),
            401 => Err(MailboxError::Login),
            _ => Err(MailboxError::Protocol(format!("HTTP {status}"))),
        }
    }

    fn set_flag(&mut self, _folder: &str, id: &str, flag: Flag, on: bool) -> Result<()> {
        let change = match flag {
            Flag::Seen => json!({ "isRead": on }),
            Flag::Flagged => json!({ "flag": { "flagStatus": if on { "flagged" } else { "notFlagged" } } }),
        };
        self.call("PATCH", &format!("/me/messages/{}", encode(id)), Some(&change)).map(drop)
    }

    fn move_to(&mut self, _folder: &str, id: &str, dest: &str) -> Result<()> {
        self.call("POST", &format!("/me/messages/{}/move", encode(id)), Some(&json!({ "destinationId": dest }))).map(drop)
    }

    fn create_folder(&mut self, parent: Option<&str>, name: &str) -> Result<()> {
        let path = match parent {
            Some(id) => format!("/me/mailFolders/{}/childFolders", encode(id)),
            None => "/me/mailFolders".to_string(),
        };
        self.call("POST", &path, Some(&json!({ "displayName": name }))).map(drop)
    }

    fn rename_folder(&mut self, folder: &str, name: &str) -> Result<()> {
        self.call("PATCH", &format!("/me/mailFolders/{}", encode(folder)), Some(&json!({ "displayName": name }))).map(drop)
    }

    fn delete_folder(&mut self, folder: &str) -> Result<()> {
        self.call("DELETE", &format!("/me/mailFolders/{}", encode(folder)), None).map(drop)
    }

    fn delete_message(&mut self, _folder: &str, id: &str) -> Result<()> {
        // Graph moves a message in Deleted Items on to the recoverable items, like Outlook.
        self.call("DELETE", &format!("/me/messages/{}", encode(id)), None).map(drop)
    }

    fn set_categories(&mut self, _folder: &str, id: &str, categories: &[String], _removed: &[String]) -> Result<()> {
        self.call("PATCH", &format!("/me/messages/{}", encode(id)), Some(&json!({ "categories": categories }))).map(drop)
    }

    fn empty_folder(&mut self, folder: &str) -> Result<()> {
        // Graph has no "empty folder": every message goes one by one, as many as `list` pages.
        let path = format!("/me/mailFolders/{}/messages?$top={PAGE}&$select=id", encode(folder));
        for m in self.list(&path)? {
            if let Some(id) = id_of(&m) {
                self.call("DELETE", &format!("/me/messages/{}", encode(&id)), None)?;
            }
        }
        Ok(())
    }

    fn append(&mut self, folder: &str, message: &[u8]) -> Result<()> {
        // Graph files raw MIME as a draft; sending goes through `send`, which files its own copy.
        self.post_mime(&format!("/me/mailFolders/{}/messages", encode(folder)), message)
    }

    fn send(&mut self, message: &[u8]) -> Result<bool> {
        self.post_mime("/me/sendMail", message)?;
        Ok(true)
    }

    fn items(&mut self) -> Option<&mut dyn ItemStore> {
        Some(self)
    }

    fn logout(self: Box<Self>) {}
}

impl ItemStore for GraphClient {
    fn appointments(&mut self, from: i64, to: i64) -> Result<Vec<Appointment>> {
        let path = format!(
            "/me/calendarView?startDateTime={}Z&endDateTime={}Z&$top={PAGE}&$select=id,subject,body,start,end,isAllDay,location,isCancelled,isReminderOn,reminderMinutesBeforeStart,sensitivity,showAs,categories,attendees,organizer,iCalUId",
            encode(&utc(from)),
            encode(&utc(to.max(from)))
        );
        Ok(self.list(&path)?.iter().filter_map(appointment_of).collect())
    }

    fn contacts(&mut self) -> Result<Vec<Contact>> {
        let path = format!(
            "/me/contacts?$top={PAGE}&$select=id,displayName,givenName,surname,emailAddresses,businessPhones,mobilePhone,homePhones,companyName,personalNotes"
        );
        Ok(self.list(&path)?.iter().filter_map(contact_of).collect())
    }

    fn tasks(&mut self) -> Result<Vec<Task>> {
        let list = self.task_list()?;
        let path =
            format!("/me/todo/lists/{}/tasks?$top={PAGE}&$select=id,title,status,importance,dueDateTime,completedDateTime,body", encode(&list));
        Ok(self.list(&path)?.iter().filter_map(task_of).collect())
    }

    fn save(&mut self, item: &Item) -> Result<String> {
        let zone = self.time_zone.clone();
        let (body, create_path, prefix) = match item {
            Item::Appointment(a) => (Self::event_json(a, &zone), "/me/events".to_string(), "event:"),
            Item::Contact(c) => (Self::contact_json(c), "/me/contacts".to_string(), "contact:"),
            Item::Task(t) => (Self::task_json(t, &zone), format!("/me/todo/lists/{}/tasks", encode(&self.task_list()?)), "task:"),
        };
        if item.id().is_empty() {
            Self::created(&self.call("POST", &create_path, Some(&body))?, prefix)
        } else {
            let path = self.path_of(item.id())?;
            self.call("PATCH", &path, Some(&body))?;
            Ok(item.id().to_string())
        }
    }

    fn delete(&mut self, id: &str) -> Result<()> {
        let path = self.path_of(id)?;
        self.call("DELETE", &path, None).map(drop)
    }
}

/// HTTPS checked by the computer's own certificate store (Keychain on macOS), with aws-lc-rs
/// like the rest of Mailcraft. ureq has no root certificates of its own in this build: an
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
    fn a_search_quotes_the_words_and_drops_the_ordering() {
        let path = search_path("AAMk=", "fac\"tuur 2026", 999).unwrap();
        assert!(path.starts_with("/me/mailFolders/AAMk%3D/messages?$search=%22fac%5C%22tuur%202026%22&$top=250&$select=id,"), "{path}");
        assert!(!path.contains("$orderby") && !path.contains("$skip"));
        assert!(search_path("AAMk=", "  ", 10).is_none());
    }

    #[test]
    fn times_and_encoding() {
        assert_eq!(parse_utc("2026-10-14T17:46:40Z"), Some(1_792_000_000));
        assert_eq!(parse_utc("2026-10-14T17:46:40.0000000"), Some(1_792_000_000));
        assert_eq!(parse_utc("later"), None);
        assert_eq!(utc(1_792_000_000), "2026-10-14T17:46:40");
        assert_eq!(encode("AAMk=/x"), "AAMk%3D%2Fx");
        assert_eq!(moment(&json!({ "dateTime": "2026-10-14T17:46:40.0000000", "timeZone": "UTC" })), Some(1_792_000_000));
    }

    #[test]
    fn a_meeting_brings_its_attendees_and_their_answers() {
        let e = json!({ "id": "E1", "subject": "Lunch", "iCalUId": "040000008200E0",
            "start": { "dateTime": "2026-10-14T17:46:40" }, "end": { "dateTime": "2026-10-14T18:46:40" },
            "organizer": { "emailAddress": { "name": "Alain", "address": "alain@example.org" } },
            "attendees": [
                { "type": "required", "status": { "response": "accepted" }, "emailAddress": { "name": "Sanne", "address": "sanne@example.org" } },
                { "type": "optional", "status": { "response": "tentativelyAccepted" }, "emailAddress": { "address": "ravi@example.org" } },
                { "type": "resource", "status": { "response": "accepted" }, "emailAddress": { "address": "zaal@example.org" } }
            ] });
        let m = appointment_of(&e).unwrap().meeting.expect("a meeting");
        assert_eq!((m.uid.as_str(), m.organizer.as_str()), ("040000008200E0", "alain@example.org"));
        let people: Vec<(&str, bool, Response)> = m.attendees.iter().map(|a| (a.email.as_str(), a.optional, a.response)).collect();
        assert_eq!(people, [("sanne@example.org", false, Response::Accepted), ("ravi@example.org", true, Response::Tentative)]);
        assert!(appointment_of(&json!({ "id": "E2", "start": e["start"], "end": e["end"], "attendees": [] })).unwrap().meeting.is_none());
    }

    #[test]
    fn messages_become_headers() {
        let m = json!({ "id": "AAMk1", "subject": "Hoi & dag", "receivedDateTime": "2026-10-14T17:46:40Z", "isRead": false,
            "hasAttachments": true, "flag": { "flagStatus": "flagged" }, "internetMessageId": "<abc@example.org>", "importance": "low",
            "from": { "emailAddress": { "name": "Sanne", "address": "sanne@example.org" } },
            "toRecipients": [{ "emailAddress": { "name": "", "address": "jij@example.org" } }] });
        let h = header_of(&m).unwrap();
        assert_eq!((h.id.as_str(), h.header.subject.as_str(), h.date), ("AAMk1", "Hoi & dag", 1_792_000_000));
        assert!(h.flagged && h.has_attachments && !h.seen && !h.answered);
        assert_eq!(h.header.from.unwrap().display(), "Sanne");
        assert_eq!(h.header.to[0].email(), "jij@example.org");
        assert_eq!(h.header.message_id.as_deref(), Some("<abc@example.org>"));
        assert_eq!(h.header.importance, Importance::Low);
        assert!(header_of(&json!({ "subject": "no id" })).is_none());
    }

    #[test]
    fn folder_kinds_come_from_the_well_known_ids() {
        let mut c = GraphClient::offline();
        c.known = vec![("in1".into(), FolderKind::Inbox)];
        assert_eq!(c.kind_of("in1", "Whatever"), FolderKind::Inbox);
        assert_eq!(c.kind_of("x", "Inbox"), FolderKind::Other, "only the server knows which folder is the inbox");
        assert_eq!(c.kind_of("x", "Archief"), FolderKind::Archive);
        assert_eq!(c.path_of("event:E1").unwrap(), "/me/events/E1");
        assert_eq!(c.path_of("contact:C=1").unwrap(), "/me/contacts/C%3D1");
        assert!(matches!(c.path_of("nope"), Err(MailboxError::BadName(_))));
        c.task_list = Some("L1".into());
        assert_eq!(c.path_of("task:T1").unwrap(), "/me/todo/lists/L1/tasks/T1");
    }

    #[test]
    fn events_become_appointments_and_back() {
        let e = json!({ "id": "E1", "subject": "Overleg", "isAllDay": false, "body": { "contentType": "text", "content": "n" },
            "start": { "dateTime": "2026-10-14T17:46:40.0000000", "timeZone": "UTC" }, "end": { "dateTime": "2026-10-14T18:46:40.0000000", "timeZone": "UTC" },
            "location": { "displayName": "Studio" }, "isReminderOn": true, "reminderMinutesBeforeStart": 45 });
        let a = appointment_of(&e).unwrap();
        assert_eq!(
            (a.id.as_str(), a.title.as_str(), a.start, a.end, a.location.as_str(), a.notes.as_str(), a.reminder),
            ("event:E1", "Overleg", 1_792_000_000, 1_792_003_600, "Studio", "n", Some(45))
        );
        let body = GraphClient::event_json(&a, "UTC");
        assert_eq!((body["isReminderOn"].as_bool(), body["reminderMinutesBeforeStart"].as_u64()), (Some(true), Some(45)));
        assert!(!a.private);
        assert_eq!(body["sensitivity"], "normal");
        let mut hidden = e.clone();
        hidden["sensitivity"] = json!("private");
        let h = appointment_of(&hidden).unwrap();
        assert!(h.private, "Privé comes back from the sensitivity");
        assert_eq!(GraphClient::event_json(&h, "UTC")["sensitivity"], "private");
        assert_eq!((a.show_as, body["showAs"].as_str()), (ShowAs::Busy, Some("busy")));
        let mut maybe = e.clone();
        maybe["showAs"] = json!("tentative");
        let m = appointment_of(&maybe).unwrap();
        assert_eq!(m.show_as, ShowAs::Tentative);
        assert_eq!(GraphClient::event_json(&m, "UTC")["showAs"], "tentative");
        assert_eq!(body["categories"], json!([]), "no categories clears them on the server");
        maybe["categories"] = json!(["Werk", "", "Klant"]);
        let tagged = appointment_of(&maybe).unwrap();
        assert_eq!(tagged.categories, vec!["Werk", "Klant"]);
        assert_eq!(GraphClient::event_json(&tagged, "UTC")["categories"], json!(["Werk", "Klant"]));
        maybe["showAs"] = json!("workingElsewhere");
        assert_eq!(appointment_of(&maybe).unwrap().show_as, ShowAs::Free, "working elsewhere leaves the time free");
        let silent = Appointment { reminder: None, ..a.clone() };
        let body = GraphClient::event_json(&silent, "UTC");
        assert_eq!(body["isReminderOn"], false);
        assert!(body.get("reminderMinutesBeforeStart").is_none());
        assert!(appointment_of(&json!({ "id": "E2", "isCancelled": true, "start": { "dateTime": "2026-10-14T17:46:40" }, "end": { "dateTime": "2026-10-14T17:46:40" } })).is_none());
        let body = GraphClient::event_json(&a, "Europe/Amsterdam");
        assert_eq!(body["start"], json!({ "dateTime": "2026-10-14T17:46:40", "timeZone": "UTC" }));
        assert_eq!(body["location"]["displayName"], "Studio");
        let whole = Appointment { all_day: true, ..a.clone() };
        let body = GraphClient::event_json(&whole, "Europe/Amsterdam");
        assert_eq!(body["start"]["timeZone"], "Europe/Amsterdam");
        assert_eq!(body["start"]["dateTime"], local(1_792_000_000));

        let weekly = Recurrence { interval: 2, weekdays: vec![0, 2], count: Some(10), ..Recurrence::every(Frequency::Weekly) };
        let series = Appointment { recurrence: Some(weekly), ..a };
        let body = GraphClient::event_json(&series, "Europe/Amsterdam");
        assert_eq!(body["start"]["timeZone"], "Europe/Amsterdam", "a series goes as wall-clock time in its zone");
        assert_eq!(
            body["recurrence"]["pattern"],
            json!({ "type": "weekly", "interval": 2, "daysOfWeek": ["monday", "wednesday"], "firstDayOfWeek": "monday" })
        );
        assert_eq!(body["recurrence"]["range"]["type"], "numbered");
        assert_eq!(body["recurrence"]["range"]["numberOfOccurrences"], 10);
        assert_eq!(body["recurrence"]["range"]["startDate"], local(1_792_000_000)[..10]);
        let yearly = Appointment { recurrence: Some(Recurrence::every(Frequency::Yearly)), ..series };
        let body = GraphClient::event_json(&yearly, "Europe/Amsterdam");
        assert_eq!(body["recurrence"]["pattern"]["type"], "absoluteYearly");
        assert_eq!(body["recurrence"]["range"]["type"], "noEnd");
    }

    #[test]
    fn contacts_and_tasks_round_trip() {
        let c = json!({ "id": "C1", "displayName": "Sanne de Vries", "givenName": "Sanne", "surname": "de Vries", "companyName": "Studio", "personalNotes": "n",
            "emailAddresses": [{ "name": "Sanne", "address": "Sanne@Example.org" }, { "address": "/o=Exchange/cn=Sanne" }],
            "businessPhones": ["020 1"], "mobilePhone": "06 1", "homePhones": [] });
        let contact = contact_of(&c).unwrap();
        assert_eq!((contact.id.as_str(), contact.name.as_str(), contact.company.as_str()), ("contact:C1", "Sanne de Vries", "Studio"));
        assert_eq!(contact.emails, vec!["sanne@example.org"]);
        assert_eq!(contact.phones, vec!["020 1", "06 1"]);
        let body = GraphClient::contact_json(&contact);
        assert_eq!((body["givenName"].as_str(), body["surname"].as_str()), (Some("Sanne"), Some("de Vries")));
        assert_eq!(body["businessPhones"], json!(["020 1"]));
        assert_eq!(body["mobilePhone"], "06 1");
        assert_eq!(body["homePhones"], json!([]));
        assert_eq!(body["emailAddresses"][0]["address"], "sanne@example.org");

        let t = json!({ "id": "T1", "title": "Offerte", "status": "completed", "importance": "high", "body": { "content": "n" },
            "dueDateTime": { "dateTime": "2026-10-13T22:00:00.0000000", "timeZone": "UTC" }, "completedDateTime": { "dateTime": "2026-10-14T17:46:40.0000000", "timeZone": "UTC" } });
        let task = task_of(&t).unwrap();
        assert_eq!(
            (task.id.as_str(), task.done, task.priority, task.due, task.completed),
            ("task:T1", true, Priority::High, Some(1_791_928_800), Some(1_792_000_000))
        );
        let body = GraphClient::task_json(&task, "Europe/Amsterdam");
        assert_eq!((body["status"].as_str(), body["importance"].as_str()), (Some("completed"), Some("high")));
        assert_eq!(body["dueDateTime"]["timeZone"], "Europe/Amsterdam");
        let open = Task { title: "x".into(), ..Task::default() };
        assert!(GraphClient::task_json(&open, "UTC")["dueDateTime"].is_null());
    }
}
