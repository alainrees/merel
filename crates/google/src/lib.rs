//! Gmail accounts: IMAP for the mail, and Google's Calendar, People and Tasks APIs for the
//! appointments, contacts and tasks, all with the one OAuth access token.
//!
//! [`GoogleMailbox`] wraps the IMAP client and offers the APIs as an [`ItemStore`] when the
//! token carries the three extra scopes ([`SCOPES`]); an account signed in before those were
//! asked for keeps working as mail only, until it signs in again. Every call is HTTPS with the
//! platform certificate verifier, ids go into URLs percent-encoded, and replies are read with
//! a size cap. A 401 is reported as a refused login, which makes the worker fetch a fresh
//! token and retry once.
//!
//! Store ids carry their kind, since the APIs are three: `event:<id>`, `people/<id>` (the
//! People API's own resource name) and `task:<id>`.

use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use mailcraft_imap::ImapClient;
use mailcraft_mailbox::{Credential, Flag, Item, ItemStore, Mailbox, MailboxError, RemoteFolder, RemoteHeader, Result};
use mailcraft_model::{Address, Appointment, Contact, Server, Task, clean_text};
use mailcraft_secrets::Secret;
use serde_json::{Value, json};

/// The scopes the item store needs, besides `https://mail.google.com/` for IMAP and SMTP.
pub const SCOPES: [&str; 3] =
    ["https://www.googleapis.com/auth/calendar", "https://www.googleapis.com/auth/contacts", "https://www.googleapis.com/auth/tasks"];

const EVENTS: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
const PEOPLE: &str = "https://people.googleapis.com/v1";
const TASKS: &str = "https://tasks.googleapis.com/tasks/v1/lists/@default/tasks";
const TOKENINFO: &str = "https://oauth2.googleapis.com/tokeninfo";
const PERSON_FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations,biographies";
/// The largest reply accepted.
const MAX_REPLY: u64 = 16 * 1024 * 1024;
/// Pages followed per listing; more items than that are not shown.
const MAX_PAGES: usize = 10;

/// A Gmail account's store: IMAP, plus the item APIs when the token allows them.
pub struct GoogleMailbox {
    imap: ImapClient,
    items: Option<GoogleItems>,
}

impl GoogleMailbox {
    /// Logs in over IMAP and, for an OAuth token with the item scopes, readies the APIs.
    pub fn connect(server: &Server, username: &str, credential: &Credential) -> Result<Self> {
        let imap = ImapClient::connect(server, username, credential).map_err(MailboxError::from)?;
        let items = match credential {
            Credential::OAuth2(token) => GoogleItems::connect(token),
            Credential::Password(_) => None,
        };
        Ok(GoogleMailbox { imap, items })
    }
}

impl Mailbox for GoogleMailbox {
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

    fn items(&mut self) -> Option<&mut dyn ItemStore> {
        self.items.as_mut().map(|i| i as &mut dyn ItemStore)
    }

    fn logout(self: Box<Self>) {
        Mailbox::logout(Box::new(self.imap));
    }
}

/// The Calendar, People and Tasks APIs behind one access token.
pub struct GoogleItems {
    agent: ureq::Agent,
    token: Secret,
}

/// Whether the space-separated `scope` of a token covers every item API.
pub fn has_scopes(scope: &str) -> bool {
    let granted: Vec<&str> = scope.split_whitespace().collect();
    SCOPES.iter().all(|s| granted.contains(s))
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

fn rfc3339(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_else(|| rfc3339(0))
}

/// The local calendar day of `ts`, as `2026-10-18`.
fn local_date(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "1970-01-01".into())
}

/// Local midnight of the day `date` (`2026-10-18`) as Unix seconds.
fn midnight_of(date: &str) -> Option<i64> {
    let d = NaiveDate::parse_from_str(date.get(..10)?, "%Y-%m-%d").ok()?;
    Local.from_local_datetime(&d.and_time(NaiveTime::MIN)).earliest().map(|d| d.timestamp())
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn title_of(v: &Value, key: &str) -> String {
    let t = clean_text(str_of(v, key));
    if t.is_empty() { "(Zonder titel)".into() } else { t }
}

/// A moment of an event: `(seconds, all day)`, from `{"date": ..}` or `{"dateTime": ..}`.
fn moment(v: &Value) -> Option<(i64, bool)> {
    if let Some(date) = v.get("date").and_then(Value::as_str) {
        return Some((midnight_of(date)?, true));
    }
    let dt = v.get("dateTime").and_then(Value::as_str)?;
    Some((DateTime::parse_from_rfc3339(dt).ok()?.timestamp(), false))
}

fn appointment_of(e: &Value) -> Option<Appointment> {
    if str_of(e, "status") == "cancelled" {
        return None;
    }
    let id = e.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())?;
    let (start, all_day) = e.get("start").and_then(moment)?;
    let (end, _) = e.get("end").and_then(moment)?;
    Appointment {
        id: format!("event:{id}"),
        calendar_id: String::new(),
        title: title_of(e, "summary"),
        start,
        end: end.max(start + 1),
        all_day,
        location: str_of(e, "location").to_string(),
        notes: str_of(e, "description").to_string(),
        recurrence: None,
        reminder: reminder_of(e),
    }
    .validated()
    .ok()
}

/// The event's own pop-up reminder, in minutes before the start. An event that uses the
/// calendar's default reminders shows none here: what those are is not in the event.
fn reminder_of(e: &Value) -> Option<u32> {
    let reminders = e.get("reminders")?;
    if reminders.get("useDefault").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let overrides = reminders.get("overrides").and_then(Value::as_array)?;
    let minutes = |o: &Value| o.get("minutes").and_then(Value::as_u64).and_then(|m| u32::try_from(m).ok());
    overrides.iter().find(|o| str_of(o, "method") == "popup").and_then(minutes).or_else(|| overrides.iter().find_map(minutes))
}

/// The event as the Calendar API takes it. The field not in use is cleared, so a `PATCH`
/// that turns a timed appointment into an all-day one (or back) takes. A reminder becomes
/// the event's own pop-up; none hands the event back to the calendar's default reminders.
fn event_json(a: &Appointment) -> Value {
    let time = |ts| {
        if a.all_day {
            json!({ "date": local_date(ts), "dateTime": null, "timeZone": null })
        } else {
            json!({ "dateTime": rfc3339(ts), "date": null })
        }
    };
    let reminders = match a.reminder {
        Some(m) => json!({ "useDefault": false, "overrides": [{ "method": "popup", "minutes": m }] }),
        None => json!({ "useDefault": true }),
    };
    json!({ "summary": a.title, "description": a.notes, "location": a.location, "start": time(a.start), "end": time(a.end), "reminders": reminders })
}

fn first_str<'a>(v: &'a Value, list: &str, key: &str) -> &'a str {
    v.get(list).and_then(Value::as_array).and_then(|l| l.first()).map(|e| str_of(e, key)).unwrap_or("")
}

fn values(v: &Value, list: &str) -> Vec<String> {
    v.get(list)
        .and_then(Value::as_array)
        .map(|l| l.iter().map(|e| str_of(e, "value").trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

fn contact_of(p: &Value) -> Option<Contact> {
    let id = p.get("resourceName").and_then(Value::as_str).filter(|id| id.starts_with("people/"))?;
    let name = first_str(p, "names", "displayName");
    let name = if name.is_empty() { first_str(p, "names", "unstructuredName") } else { name };
    Contact {
        id: id.to_string(),
        name: name.to_string(),
        emails: values(p, "emailAddresses").into_iter().filter(|e| Address::new(None, e).is_ok()).collect(),
        phones: values(p, "phoneNumbers"),
        company: first_str(p, "organizations", "name").to_string(),
        notes: first_str(p, "biographies", "value").to_string(),
        account_id: None,
    }
    .validated()
    .ok()
}

/// The person as the People API takes it.
fn person_json(c: &Contact) -> Value {
    let (given, family) = c.name.split_once(' ').unwrap_or((c.name.as_str(), ""));
    let organizations: Vec<Value> = if c.company.is_empty() { Vec::new() } else { vec![json!({ "name": c.company })] };
    let biographies: Vec<Value> = if c.notes.is_empty() { Vec::new() } else { vec![json!({ "value": c.notes, "contentType": "TEXT_PLAIN" })] };
    json!({
        "names": [{ "givenName": given, "familyName": family.trim() }],
        "emailAddresses": c.emails.iter().map(|e| json!({ "value": e })).collect::<Vec<_>>(),
        "phoneNumbers": c.phones.iter().map(|p| json!({ "value": p })).collect::<Vec<_>>(),
        "organizations": organizations,
        "biographies": biographies,
    })
}

fn task_of(t: &Value) -> Option<Task> {
    let id = t.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())?;
    let done = str_of(t, "status") == "completed";
    Task {
        id: format!("task:{id}"),
        title: title_of(t, "title"),
        notes: str_of(t, "notes").to_string(),
        // A due day comes as midnight UTC of that day; the date is what counts.
        due: t.get("due").and_then(Value::as_str).and_then(midnight_of),
        priority: Default::default(),
        done,
        completed: t.get("completed").and_then(Value::as_str).and_then(|c| DateTime::parse_from_rfc3339(c).ok()).map(|d| d.timestamp()),
        account_id: None,
    }
    .validated()
    .ok()
}

/// The task as the Tasks API takes it; Google keeps no priority, so that stays here.
fn task_json(t: &Task) -> Value {
    let completed = t.done.then(|| rfc3339(t.completed.unwrap_or_else(|| Utc::now().timestamp())));
    json!({
        "title": t.title,
        "notes": t.notes,
        "due": t.due.map(|d| format!("{}T00:00:00.000Z", local_date(d))),
        "status": if t.done { "completed" } else { "needsAction" },
        "completed": completed,
    })
}

/// Drops `null` members, for a `POST` that should not name what it does not set.
fn without_nulls(mut v: Value) -> Value {
    if let Value::Object(map) = &mut v {
        map.retain(|_, v| !v.is_null());
        for v in map.values_mut() {
            if let Value::Object(inner) = v {
                inner.retain(|_, v| !v.is_null());
            }
        }
    }
    v
}

/// Where an item's id leads: the URL that reads, changes or deletes it.
fn url_of(id: &str) -> Result<String> {
    if let Some(event) = id.strip_prefix("event:") {
        Ok(format!("{EVENTS}/{}", encode(event)))
    } else if let Some(person) = id.strip_prefix("people/") {
        Ok(format!("{PEOPLE}/people/{}", encode(person)))
    } else if let Some(task) = id.strip_prefix("task:") {
        Ok(format!("{TASKS}/{}", encode(task)))
    } else {
        Err(MailboxError::BadName(id.chars().take(40).collect()))
    }
}

impl GoogleItems {
    /// The store, when `token` carries the item scopes; `None` (and a log line) otherwise, or
    /// when Google cannot say.
    pub fn connect(token: &Secret) -> Option<Self> {
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder().timeout_global(Some(Duration::from_secs(60))).http_status_as_error(false).user_agent("Mailcraft").build(),
        );
        let store = GoogleItems { agent, token: token.clone() };
        match store.scopes() {
            Ok(scope) if has_scopes(&scope) => Some(store),
            Ok(_) => {
                log::info!("the Google token has no calendar, contacts or tasks scope; mail only until the account signs in again");
                None
            }
            Err(e) => {
                log::warn!("could not check the Google token's scopes: {e}");
                None
            }
        }
    }

    /// The scopes of the token, as Google reports them.
    fn scopes(&self) -> Result<String> {
        let mut response = self
            .agent
            .post(TOKENINFO)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(format!("access_token={}", encode(self.token.expose())).as_bytes())
            .map_err(|e| MailboxError::Connect { host: "oauth2.googleapis.com".into(), reason: e.to_string() })?;
        let info: Value = response.body_mut().with_config().limit(64 * 1024).read_json().map_err(|e| MailboxError::Protocol(e.to_string()))?;
        if response.status().as_u16() != 200 {
            return Err(MailboxError::Protocol(clean_text(str_of(&info, "error_description"))));
        }
        Ok(str_of(&info, "scope").to_string())
    }

    /// One API call. `Ok(Null)` for an empty reply.
    fn call(&self, method: &str, url: &str, body: Option<&Value>) -> Result<Value> {
        let host = url.split('/').nth(2).unwrap_or("googleapis.com").to_string();
        let connect = |e: ureq::Error| MailboxError::Connect { host: host.clone(), reason: e.to_string() };
        let auth = format!("Bearer {}", self.token.expose());
        let mut response = match (method, body) {
            ("GET", _) => self.agent.get(url).header("Authorization", &auth).call().map_err(connect)?,
            ("DELETE", _) => self.agent.delete(url).header("Authorization", &auth).call().map_err(connect)?,
            (_, Some(body)) => {
                let req = if method == "PATCH" { self.agent.patch(url) } else { self.agent.post(url) };
                req.header("Authorization", &auth).header("Content-Type", "application/json").send(body.to_string().as_bytes()).map_err(connect)?
            }
            (m, None) => return Err(MailboxError::Protocol(format!("{m} without a body"))),
        };
        let status = response.status().as_u16();
        let text = response.body_mut().with_config().limit(MAX_REPLY).read_to_string().map_err(|e| MailboxError::Protocol(e.to_string()))?;
        let value: Value = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::Null) };
        match status {
            200..=299 => Ok(value),
            401 => Err(MailboxError::Login),
            // Deleting what is already gone is done.
            404 | 410 if method == "DELETE" => Ok(Value::Null),
            _ => {
                let message = value.get("error").map(|e| str_of(e, "message")).filter(|m| !m.is_empty()).map(clean_text);
                Err(MailboxError::Protocol(message.unwrap_or_else(|| format!("HTTP {status}"))))
            }
        }
    }

    /// Every item of a paged listing, following `nextPageToken` up to `MAX_PAGES` pages.
    fn list(&self, url: &str, key: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page_url = match &token {
                Some(t) => format!("{url}&pageToken={}", encode(t)),
                None => url.to_string(),
            };
            let page = self.call("GET", &page_url, None)?;
            if let Some(items) = page.get(key).and_then(Value::as_array) {
                out.extend(items.iter().cloned());
            }
            token = page.get("nextPageToken").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
            if token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn created_id(reply: &Value, key: &str, prefix: &str) -> Result<String> {
        reply
            .get(key)
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(|id| format!("{prefix}{id}"))
            .ok_or_else(|| MailboxError::Protocol("Google did not return the new item's id".into()))
    }
}

impl ItemStore for GoogleItems {
    fn appointments(&mut self, from: i64, to: i64) -> Result<Vec<Appointment>> {
        let url = format!(
            "{EVENTS}?singleEvents=true&orderBy=startTime&maxResults=2500&timeMin={}&timeMax={}",
            encode(&rfc3339(from)),
            encode(&rfc3339(to.max(from)))
        );
        Ok(self.list(&url, "items")?.iter().filter_map(appointment_of).collect())
    }

    fn contacts(&mut self) -> Result<Vec<Contact>> {
        let url = format!("{PEOPLE}/people/me/connections?personFields={PERSON_FIELDS}&pageSize=1000");
        Ok(self.list(&url, "connections")?.iter().filter_map(contact_of).collect())
    }

    fn tasks(&mut self) -> Result<Vec<Task>> {
        let url = format!("{TASKS}?maxResults=100&showCompleted=true&showHidden=true");
        Ok(self.list(&url, "items")?.iter().filter_map(task_of).collect())
    }

    fn save(&mut self, item: &Item) -> Result<String> {
        let id = item.id();
        match item {
            Item::Appointment(a) => {
                let body = event_json(a);
                if id.is_empty() {
                    Self::created_id(&self.call("POST", EVENTS, Some(&without_nulls(body)))?, "id", "event:")
                } else {
                    self.call("PATCH", &url_of(id)?, Some(&body))?;
                    Ok(id.to_string())
                }
            }
            Item::Contact(c) => {
                let mut body = person_json(c);
                if id.is_empty() {
                    return Self::created_id(&self.call("POST", &format!("{PEOPLE}/people:createContact"), Some(&body))?, "resourceName", "");
                }
                // An update must carry the current etag, so the person is read first.
                let url = url_of(id)?;
                let current = self.call("GET", &format!("{url}?personFields=names"), None)?;
                body["etag"] = current.get("etag").cloned().unwrap_or(Value::Null);
                body["resourceName"] = Value::String(id.to_string());
                self.call("PATCH", &format!("{url}:updateContact?updatePersonFields={PERSON_FIELDS}"), Some(&body))?;
                Ok(id.to_string())
            }
            Item::Task(t) => {
                let body = task_json(t);
                if id.is_empty() {
                    Self::created_id(&self.call("POST", TASKS, Some(&without_nulls(body)))?, "id", "task:")
                } else {
                    self.call("PATCH", &url_of(id)?, Some(&body))?;
                    Ok(id.to_string())
                }
            }
        }
    }

    fn delete(&mut self, id: &str) -> Result<()> {
        let url = url_of(id)?;
        let url = if id.starts_with("people/") { format!("{url}:deleteContact") } else { url };
        self.call("DELETE", &url, None).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_and_encoding() {
        assert!(has_scopes(
            "https://mail.google.com/ https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/contacts https://www.googleapis.com/auth/tasks"
        ));
        assert!(!has_scopes("https://mail.google.com/ https://www.googleapis.com/auth/calendar"));
        assert_eq!(encode("a b/c:d_e"), "a%20b%2Fc%3Ad_e");
        assert_eq!(rfc3339(1_792_000_000), "2026-10-14T17:46:40Z");
    }

    #[test]
    fn ids_route_to_their_api() {
        assert_eq!(url_of("event:abc_20261018T090000Z").unwrap(), format!("{EVENTS}/abc_20261018T090000Z"));
        assert_eq!(url_of("people/c123").unwrap(), format!("{PEOPLE}/people/c123"));
        assert_eq!(url_of("task:MTIz").unwrap(), format!("{TASKS}/MTIz"));
        assert!(matches!(url_of("AAMk"), Err(MailboxError::BadName(_))));
        assert!(matches!(url_of("event:../x"), Ok(u) if u.ends_with("/..%2Fx")));
    }

    #[test]
    fn events_become_appointments_and_back() {
        let timed = json!({ "id": "e1", "summary": "Overleg", "location": "Studio", "description": "d",
            "start": { "dateTime": "2026-10-14T19:46:40+02:00" }, "end": { "dateTime": "2026-10-14T20:46:40+02:00" },
            "reminders": { "useDefault": false, "overrides": [{ "method": "email", "minutes": 1440 }, { "method": "popup", "minutes": 20 }] } });
        let a = appointment_of(&timed).unwrap();
        assert_eq!((a.id.as_str(), a.title.as_str(), a.start, a.end, a.all_day), ("event:e1", "Overleg", 1_792_000_000, 1_792_003_600, false));
        assert_eq!(a.reminder, Some(20), "the pop-up counts, not the e-mail");
        assert_eq!(event_json(&a)["reminders"], json!({ "useDefault": false, "overrides": [{ "method": "popup", "minutes": 20 }] }));
        let whole = json!({ "id": "e2", "start": { "date": "2026-10-18" }, "end": { "date": "2026-10-19" }, "reminders": { "useDefault": true } });
        let a = appointment_of(&whole).unwrap();
        assert_eq!((a.title.as_str(), a.all_day, a.end - a.start, a.reminder), ("(Zonder titel)", true, 86_400, None));
        assert_eq!(event_json(&a)["reminders"], json!({ "useDefault": true }), "no reminder here leaves the calendar's defaults");
        assert_eq!(a.start, midnight_of("2026-10-18").unwrap());
        assert!(
            appointment_of(&json!({ "id": "e3", "status": "cancelled", "start": { "date": "2026-10-18" }, "end": { "date": "2026-10-19" } }))
                .is_none()
        );

        let body = event_json(&a);
        assert_eq!(body["start"]["date"], "2026-10-18");
        assert!(body["start"]["dateTime"].is_null());
        let stripped = without_nulls(body);
        assert!(stripped["start"].get("dateTime").is_none() && stripped["end"]["date"] == "2026-10-19");
        let mut timed = a.clone();
        timed.all_day = false;
        let body = event_json(&timed);
        assert!(body["start"]["dateTime"].as_str().unwrap().ends_with('Z') && body["start"]["date"].is_null());
    }

    #[test]
    fn people_become_contacts_and_back() {
        let p = json!({ "resourceName": "people/c1", "etag": "x",
            "names": [{ "displayName": "Sanne de Vries", "givenName": "Sanne" }],
            "emailAddresses": [{ "value": "Sanne@Example.org" }, { "value": "not an address" }],
            "phoneNumbers": [{ "value": "06 1" }], "organizations": [{ "name": "Studio" }], "biographies": [{ "value": "n" }] });
        let c = contact_of(&p).unwrap();
        assert_eq!((c.id.as_str(), c.name.as_str(), c.company.as_str(), c.notes.as_str()), ("people/c1", "Sanne de Vries", "Studio", "n"));
        assert_eq!((c.emails.as_slice(), c.phones.as_slice()), (&["sanne@example.org".to_string()][..], &["06 1".to_string()][..]));
        assert!(contact_of(&json!({ "resourceName": "otherContacts/c1" })).is_none());
        let body = person_json(&c);
        assert_eq!(body["names"][0]["givenName"], "Sanne");
        assert_eq!(body["names"][0]["familyName"], "de Vries");
        assert_eq!(body["emailAddresses"][0]["value"], "sanne@example.org");
        let plain = Contact { name: "Ravi".into(), ..Contact::default() };
        let body = person_json(&plain);
        assert_eq!(body["names"][0]["familyName"], "");
        assert!(body["organizations"].as_array().unwrap().is_empty());
    }

    #[test]
    fn tasks_become_tasks_and_back() {
        let t = json!({ "id": "t1", "title": "Offerte", "notes": "n", "status": "completed", "due": "2026-10-18T00:00:00.000Z", "completed": "2026-10-14T17:46:40.000Z" });
        let task = task_of(&t).unwrap();
        assert_eq!((task.id.as_str(), task.done, task.due, task.completed), ("task:t1", true, midnight_of("2026-10-18"), Some(1_792_000_000)));
        let body = task_json(&task);
        assert_eq!(body["due"], "2026-10-18T00:00:00.000Z");
        assert_eq!(body["status"], "completed");
        let open = task_of(&json!({ "id": "t2", "title": "x", "status": "needsAction" })).unwrap();
        let body = task_json(&open);
        assert!(body["due"].is_null() && body["completed"].is_null() && body["status"] == "needsAction");
        assert!(without_nulls(body).get("due").is_none());
    }
}
