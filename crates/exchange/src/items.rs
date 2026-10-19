//! The calendar, contacts and tasks of an Exchange mailbox, as an [`ItemStore`].
//!
//! Listing is two requests: `FindItem` for the ids (a `CalendarView` for appointments, which
//! expands repeating ones into occurrences) and `GetItem` in batches for the fields, body
//! included. Saving is `CreateItem` into the well-known folder or `UpdateItem` field by field;
//! fields the item no longer has are cleared in a second, lenient `UpdateItem`, because a
//! server may refuse to delete a property the item never had.
//!
//! Exchange keeps all-day appointments and due days as midnight in a time zone. The request
//! header names the computer's zone when [`windows_zone`] knows its Windows name, so a day
//! saved here is the same day in Outlook; the engine snaps what comes back to local midnight.

use chrono::{DateTime, Local, TimeZone, Utc};
use mailcraft_mailbox::{Item, ItemStore, MailboxError, Result};
use mailcraft_model::{Address, Appointment, Contact, Priority, Task, clean_text};
use roxmltree::Node;

use crate::{EwsClient, child, esc, find, parse, parse_datetime, response_messages, text};

/// Items returned per listing; a mailbox with more just shows the first ones.
const PAGE: u32 = 1000;
/// Ids per `GetItem` request.
const BATCH: usize = 50;

const EMAIL_KEYS: [&str; 3] = ["EmailAddress1", "EmailAddress2", "EmailAddress3"];
const PHONE_KEYS: [&str; 3] = ["BusinessPhone", "MobilePhone", "HomePhone"];

/// `ts` as `2026-10-18T09:00:00Z`.
fn utc(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_else(|| utc(0))
}

/// `ts` as local wall-clock time with its offset, `2026-10-18T00:00:00+02:00`: what a midnight
/// should look like to a server that thinks in the same time zone.
fn local(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).single().map(|d| d.format("%Y-%m-%dT%H:%M:%S%:z").to_string()).unwrap_or_else(|| utc(ts))
}

fn item_id(node: Node<'_, '_>) -> Option<String> {
    child(node, "ItemId").and_then(|n| n.attribute("Id")).filter(|id| !id.is_empty()).map(str::to_string)
}

fn body_text(node: Node<'_, '_>) -> String {
    child(node, "Body").and_then(|b| b.text()).unwrap_or("").to_string()
}

fn subject(node: Node<'_, '_>) -> String {
    let s = text(node, "Subject").map(clean_text).unwrap_or_default();
    if s.is_empty() { "(Zonder titel)".into() } else { s }
}

fn appointment_of(node: Node<'_, '_>) -> Option<Appointment> {
    let start = text(node, "Start").and_then(parse_datetime)?;
    let end = text(node, "End").and_then(parse_datetime)?;
    Appointment {
        id: item_id(node)?,
        calendar_id: String::new(),
        title: subject(node),
        start,
        end: end.max(start + 1),
        all_day: text(node, "IsAllDayEvent") == Some("true"),
        location: text(node, "Location").unwrap_or("").to_string(),
        notes: body_text(node),
        recurrence: None,
        reminder: (text(node, "ReminderIsSet") == Some("true")).then(|| text(node, "ReminderMinutesBeforeStart")?.trim().parse().ok()).flatten(),
    }
    .validated()
    .ok()
}

/// The `Entry` texts of a `name`d collection (`EmailAddresses`, `PhoneNumbers`), in order.
fn entries<'a>(node: Node<'a, 'a>, name: &str) -> Vec<(Option<&'a str>, &'a str)> {
    child(node, name)
        .map(|list| {
            list.children()
                .filter(|n| n.is_element() && n.tag_name().name() == "Entry")
                .filter_map(|e| Some((e.attribute("RoutingType"), e.text()?.trim())))
                .filter(|(_, v)| !v.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn contact_of(node: Node<'_, '_>) -> Option<Contact> {
    let given = text(node, "GivenName").unwrap_or("");
    let surname = text(node, "Surname").unwrap_or("");
    let name = text(node, "DisplayName").filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{given} {surname}"));
    // Addresses inside the organisation may come as X.500 names; only SMTP ones are of use.
    let emails = entries(node, "EmailAddresses")
        .into_iter()
        .filter(|(routing, v)| *routing != Some("EX") && Address::new(None, v).is_ok())
        .map(|(_, v)| v.to_string())
        .collect();
    Contact {
        id: item_id(node)?,
        name,
        emails,
        phones: entries(node, "PhoneNumbers").into_iter().map(|(_, v)| v.to_string()).collect(),
        company: text(node, "CompanyName").unwrap_or("").to_string(),
        notes: body_text(node),
        account_id: None,
    }
    .validated()
    .ok()
}

fn task_of(node: Node<'_, '_>) -> Option<Task> {
    let done = text(node, "Status") == Some("Completed") || text(node, "IsComplete") == Some("true");
    Task {
        id: item_id(node)?,
        title: subject(node),
        notes: body_text(node),
        due: text(node, "DueDate").and_then(parse_datetime),
        priority: match text(node, "Importance") {
            Some("Low") => Priority::Low,
            Some("High") => Priority::High,
            _ => Priority::Normal,
        },
        done,
        completed: text(node, "CompleteDate").and_then(parse_datetime),
        account_id: None,
    }
    .validated()
    .ok()
}

/// `<t:SetItemField>` for one field of an item of `kind`.
fn set(uri: &str, kind: &str, inner: &str) -> String {
    format!("<t:SetItemField><t:FieldURI FieldURI=\"{uri}\"/><t:{kind}>{inner}</t:{kind}></t:SetItemField>")
}

fn set_indexed(uri: &str, index: &str, kind: &str, inner: &str) -> String {
    format!("<t:SetItemField><t:IndexedFieldURI FieldURI=\"{uri}\" FieldIndex=\"{index}\"/><t:{kind}>{inner}</t:{kind}></t:SetItemField>")
}

fn clear(uri: &str) -> String {
    format!("<t:DeleteItemField><t:FieldURI FieldURI=\"{uri}\"/></t:DeleteItemField>")
}

fn clear_indexed(uri: &str, index: &str) -> String {
    format!("<t:DeleteItemField><t:IndexedFieldURI FieldURI=\"{uri}\" FieldIndex=\"{index}\"/></t:DeleteItemField>")
}

/// The fields of an item as the server wants them: in schema order, with the element for each
/// (`None` to clear it).
struct Fields {
    kind: &'static str,
    /// `(field URI, index, element)`; `None` for an element clears the field.
    list: Vec<(&'static str, Option<&'static str>, Option<String>)>,
}

impl Fields {
    fn new(kind: &'static str) -> Self {
        Fields { kind, list: Vec::new() }
    }

    fn put(&mut self, uri: &'static str, element: String) {
        self.list.push((uri, None, Some(element)));
    }

    /// A text field that is cleared when empty.
    fn put_or_clear(&mut self, uri: &'static str, element: &str, value: &str) {
        let v = if value.is_empty() { None } else { Some(format!("<t:{element}>{}</t:{element}>", esc(value))) };
        self.list.push((uri, None, v));
    }

    /// The body of a `CreateItem`: the item with every field it has.
    fn create(&self) -> String {
        let inner: String = self.list.iter().filter_map(|(_, _, e)| e.as_deref()).collect();
        format!("<t:{k}>{inner}</t:{k}>", k = self.kind)
    }

    /// The updates of an `UpdateItem`: what to set, and separately what to clear.
    fn changes(&self) -> (String, String) {
        let mut sets = String::new();
        let mut clears = String::new();
        for (uri, index, element) in &self.list {
            match (index, element) {
                (None, Some(e)) => sets.push_str(&set(uri, self.kind, e)),
                (Some(i), Some(e)) => sets.push_str(&set_indexed(uri, i, self.kind, e)),
                (None, None) => clears.push_str(&clear(uri)),
                (Some(i), None) => clears.push_str(&clear_indexed(uri, i)),
            }
        }
        (sets, clears)
    }
}

fn appointment_fields(a: &Appointment) -> Fields {
    let time = |ts| if a.all_day { local(ts) } else { utc(ts) };
    let mut f = Fields::new("CalendarItem");
    f.put("item:Subject", format!("<t:Subject>{}</t:Subject>", esc(&a.title)));
    f.put("item:Body", format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&a.notes)));
    f.put("item:ReminderIsSet", format!("<t:ReminderIsSet>{}</t:ReminderIsSet>", a.reminder.is_some()));
    if let Some(m) = a.reminder {
        f.put("item:ReminderMinutesBeforeStart", format!("<t:ReminderMinutesBeforeStart>{m}</t:ReminderMinutesBeforeStart>"));
    }
    f.put("calendar:Start", format!("<t:Start>{}</t:Start>", time(a.start)));
    f.put("calendar:End", format!("<t:End>{}</t:End>", time(a.end)));
    f.put("calendar:IsAllDayEvent", format!("<t:IsAllDayEvent>{}</t:IsAllDayEvent>", a.all_day));
    f.put("calendar:Location", format!("<t:Location>{}</t:Location>", esc(&a.location)));
    f
}

fn contact_fields(c: &Contact) -> Fields {
    let (given, surname) = c.name.split_once(' ').unwrap_or((c.name.as_str(), ""));
    let mut f = Fields::new("Contact");
    f.put("item:Body", format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&c.notes)));
    f.put_or_clear("contacts:GivenName", "GivenName", given);
    f.put_or_clear("contacts:CompanyName", "CompanyName", &c.company);
    let entry = |list: &str, key: &str, value: &str| format!("<t:{list}><t:Entry Key=\"{key}\">{}</t:Entry></t:{list}>", esc(value));
    for (i, key) in EMAIL_KEYS.iter().enumerate() {
        let value = c.emails.get(i).map(|v| entry("EmailAddresses", key, v));
        f.list.push(("contacts:EmailAddress", Some(key), value));
    }
    for (i, key) in PHONE_KEYS.iter().enumerate() {
        let value = c.phones.get(i).map(|v| entry("PhoneNumbers", key, v));
        f.list.push(("contacts:PhoneNumber", Some(key), value));
    }
    f.put_or_clear("contacts:Surname", "Surname", surname.trim());
    f
}

fn task_fields(t: &Task) -> Fields {
    let mut f = Fields::new("Task");
    f.put("item:Subject", format!("<t:Subject>{}</t:Subject>", esc(&t.title)));
    f.put("item:Body", format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&t.notes)));
    let importance = match t.priority {
        Priority::Low => "Low",
        Priority::Normal => "Normal",
        Priority::High => "High",
    };
    f.put("item:Importance", format!("<t:Importance>{importance}</t:Importance>"));
    f.list.push(("task:DueDate", None, t.due.map(|d| format!("<t:DueDate>{}</t:DueDate>", local(d)))));
    f.put("task:Status", format!("<t:Status>{}</t:Status>", if t.done { "Completed" } else { "NotStarted" }));
    f
}

/// Where an item is created, how, and what to ask for when reading it.
fn plan(item: &Item) -> (&'static str, &'static str, Fields) {
    match item {
        Item::Appointment(a) => ("calendar", "SendMeetingInvitations=\"SendToNone\"", appointment_fields(a)),
        Item::Contact(c) => ("contacts", "MessageDisposition=\"SaveOnly\"", contact_fields(c)),
        Item::Task(t) => ("tasks", "MessageDisposition=\"SaveOnly\"", task_fields(t)),
    }
}

fn field_uris(uris: &[&str]) -> String {
    uris.iter().map(|u| format!("<t:FieldURI FieldURI=\"{u}\"/>")).collect()
}

impl EwsClient {
    /// The ids of the items `view` selects in the well-known folder `folder`.
    fn list_ids(&self, folder: &str, view: &str) -> Result<Vec<String>> {
        let xml = self.call(&format!(
            concat!(
                "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>{view}",
                "<m:ParentFolderIds><t:DistinguishedFolderId Id=\"{folder}\"/></m:ParentFolderIds></m:FindItem>"
            ),
            view = view,
            folder = folder
        ))?;
        let doc = parse(&xml)?;
        response_messages(&doc, false)?;
        let Some(items) = find(doc.root_element(), "Items") else { return Ok(Vec::new()) };
        Ok(items.children().filter(|n| n.is_element()).filter_map(item_id).collect())
    }

    /// Fetches `ids` in batches with the text body and `props`, turning each item into a `T`;
    /// an item the server cannot return (deleted meanwhile) is skipped.
    fn fetch<T>(&self, ids: &[String], props: &str, of: fn(Node<'_, '_>) -> Option<T>) -> Result<Vec<T>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
            let xml = self.call(&format!(
                concat!(
                    "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:BodyType>Text</t:BodyType>",
                    "<t:AdditionalProperties>{props}</t:AdditionalProperties></m:ItemShape><m:ItemIds>{list}</m:ItemIds></m:GetItem>"
                ),
                props = props,
                list = list
            ))?;
            let doc = parse(&xml)?;
            for m in response_messages(&doc, true)?.into_iter().flatten() {
                if let Some(items) = find(m, "Items") {
                    out.extend(items.children().filter(|n| n.is_element()).filter_map(of));
                }
            }
        }
        Ok(out)
    }

    fn create_item(&self, folder: &str, attrs: &str, item: &str) -> Result<String> {
        let xml = self.call(&format!(
            "<m:CreateItem {attrs}><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"{folder}\"/></m:SavedItemFolderId><m:Items>{item}</m:Items></m:CreateItem>"
        ))?;
        let doc = parse(&xml)?;
        response_messages(&doc, false)?;
        find(doc.root_element(), "ItemId")
            .and_then(|n| n.attribute("Id"))
            .map(str::to_string)
            .ok_or_else(|| MailboxError::Protocol("the server did not return the new item's id".into()))
    }
}

impl ItemStore for EwsClient {
    fn appointments(&mut self, from: i64, to: i64) -> Result<Vec<Appointment>> {
        let view = format!("<m:CalendarView MaxEntriesReturned=\"{PAGE}\" StartDate=\"{}\" EndDate=\"{}\"/>", utc(from), utc(to.max(from)));
        let ids = self.list_ids("calendar", &view)?;
        let props = field_uris(&[
            "item:Subject",
            "item:Body",
            "item:ReminderIsSet",
            "item:ReminderMinutesBeforeStart",
            "calendar:Start",
            "calendar:End",
            "calendar:IsAllDayEvent",
            "calendar:Location",
        ]);
        self.fetch(&ids, &props, appointment_of)
    }

    fn contacts(&mut self) -> Result<Vec<Contact>> {
        let view = format!("<m:IndexedPageItemView MaxEntriesReturned=\"{PAGE}\" Offset=\"0\" BasePoint=\"Beginning\"/>");
        let ids = self.list_ids("contacts", &view)?;
        let props = field_uris(&[
            "item:Body",
            "contacts:DisplayName",
            "contacts:GivenName",
            "contacts:Surname",
            "contacts:CompanyName",
            "contacts:EmailAddresses",
            "contacts:PhoneNumbers",
        ]);
        self.fetch(&ids, &props, contact_of)
    }

    fn tasks(&mut self) -> Result<Vec<Task>> {
        let view = format!("<m:IndexedPageItemView MaxEntriesReturned=\"{PAGE}\" Offset=\"0\" BasePoint=\"Beginning\"/>");
        let ids = self.list_ids("tasks", &view)?;
        let props =
            field_uris(&["item:Subject", "item:Body", "item:Importance", "task:DueDate", "task:Status", "task:IsComplete", "task:CompleteDate"]);
        self.fetch(&ids, &props, task_of)
    }

    fn save(&mut self, item: &Item) -> Result<String> {
        let (folder, attrs, fields) = plan(item);
        if item.id().is_empty() {
            return self.create_item(folder, attrs, &fields.create());
        }
        let (sets, clears) = fields.changes();
        self.update(item.id(), &sets)?;
        if !clears.is_empty()
            && let Err(e) = self.update(item.id(), &clears)
        {
            // A field the item never had cannot be cleared; what was set is saved all the same.
            log::debug!("{}: clearing fields of {}: {e}", self.host, item.id());
        }
        Ok(item.id().to_string())
    }

    fn delete(&mut self, id: &str) -> Result<()> {
        let xml = self.call(&format!(
            concat!(
                "<m:DeleteItem DeleteType=\"MoveToDeletedItems\" SendMeetingCancellations=\"SendToNone\" AffectedTaskOccurrences=\"AllOccurrences\">",
                "<m:ItemIds><t:ItemId Id=\"{id}\"/></m:ItemIds></m:DeleteItem>"
            ),
            id = esc(id)
        ))?;
        response_messages(&parse(&xml)?, false).map(drop)
    }
}

/// The Windows name of the IANA time zone `iana`, as Exchange wants it in a request; `None`
/// for a zone this table does not know.
pub fn windows_zone(iana: &str) -> Option<&'static str> {
    const ZONES: &[(&str, &str)] = &[
        ("Europe/Amsterdam", "W. Europe Standard Time"),
        ("Europe/Berlin", "W. Europe Standard Time"),
        ("Europe/Brussels", "Romance Standard Time"),
        ("Europe/Paris", "Romance Standard Time"),
        ("Europe/Madrid", "Romance Standard Time"),
        ("Europe/Copenhagen", "Romance Standard Time"),
        ("Europe/Luxembourg", "W. Europe Standard Time"),
        ("Europe/Vienna", "W. Europe Standard Time"),
        ("Europe/Zurich", "W. Europe Standard Time"),
        ("Europe/Rome", "W. Europe Standard Time"),
        ("Europe/Stockholm", "W. Europe Standard Time"),
        ("Europe/Oslo", "W. Europe Standard Time"),
        ("Europe/Prague", "Central Europe Standard Time"),
        ("Europe/Budapest", "Central Europe Standard Time"),
        ("Europe/Warsaw", "Central European Standard Time"),
        ("Europe/Zagreb", "Central European Standard Time"),
        ("Europe/London", "GMT Standard Time"),
        ("Europe/Dublin", "GMT Standard Time"),
        ("Europe/Lisbon", "GMT Standard Time"),
        ("Atlantic/Reykjavik", "Greenwich Standard Time"),
        ("Europe/Athens", "GTB Standard Time"),
        ("Europe/Bucharest", "GTB Standard Time"),
        ("Europe/Helsinki", "FLE Standard Time"),
        ("Europe/Kiev", "FLE Standard Time"),
        ("Europe/Kyiv", "FLE Standard Time"),
        ("Europe/Riga", "FLE Standard Time"),
        ("Europe/Vilnius", "FLE Standard Time"),
        ("Europe/Tallinn", "FLE Standard Time"),
        ("Europe/Sofia", "FLE Standard Time"),
        ("Europe/Istanbul", "Turkey Standard Time"),
        ("Europe/Moscow", "Russian Standard Time"),
        ("Africa/Johannesburg", "South Africa Standard Time"),
        ("Africa/Cairo", "Egypt Standard Time"),
        ("Africa/Lagos", "W. Central Africa Standard Time"),
        ("Africa/Nairobi", "E. Africa Standard Time"),
        ("Asia/Jerusalem", "Israel Standard Time"),
        ("Asia/Dubai", "Arabian Standard Time"),
        ("Asia/Riyadh", "Arab Standard Time"),
        ("Asia/Karachi", "Pakistan Standard Time"),
        ("Asia/Kolkata", "India Standard Time"),
        ("Asia/Calcutta", "India Standard Time"),
        ("Asia/Dhaka", "Bangladesh Standard Time"),
        ("Asia/Bangkok", "SE Asia Standard Time"),
        ("Asia/Jakarta", "SE Asia Standard Time"),
        ("Asia/Singapore", "Singapore Standard Time"),
        ("Asia/Kuala_Lumpur", "Singapore Standard Time"),
        ("Asia/Hong_Kong", "China Standard Time"),
        ("Asia/Shanghai", "China Standard Time"),
        ("Asia/Taipei", "Taipei Standard Time"),
        ("Asia/Manila", "Singapore Standard Time"),
        ("Asia/Seoul", "Korea Standard Time"),
        ("Asia/Tokyo", "Tokyo Standard Time"),
        ("Australia/Perth", "W. Australia Standard Time"),
        ("Australia/Adelaide", "Cen. Australia Standard Time"),
        ("Australia/Brisbane", "E. Australia Standard Time"),
        ("Australia/Sydney", "AUS Eastern Standard Time"),
        ("Australia/Melbourne", "AUS Eastern Standard Time"),
        ("Pacific/Auckland", "New Zealand Standard Time"),
        ("America/Sao_Paulo", "E. South America Standard Time"),
        ("America/Argentina/Buenos_Aires", "Argentina Standard Time"),
        ("America/Santiago", "Pacific SA Standard Time"),
        ("America/Bogota", "SA Pacific Standard Time"),
        ("America/Lima", "SA Pacific Standard Time"),
        ("America/Caracas", "Venezuela Standard Time"),
        ("America/Mexico_City", "Central Standard Time (Mexico)"),
        ("America/Halifax", "Atlantic Standard Time"),
        ("America/New_York", "Eastern Standard Time"),
        ("America/Toronto", "Eastern Standard Time"),
        ("America/Detroit", "Eastern Standard Time"),
        ("America/Chicago", "Central Standard Time"),
        ("America/Winnipeg", "Central Standard Time"),
        ("America/Denver", "Mountain Standard Time"),
        ("America/Edmonton", "Mountain Standard Time"),
        ("America/Phoenix", "US Mountain Standard Time"),
        ("America/Los_Angeles", "Pacific Standard Time"),
        ("America/Vancouver", "Pacific Standard Time"),
        ("America/Anchorage", "Alaskan Standard Time"),
        ("Pacific/Honolulu", "Hawaiian Standard Time"),
        ("America/Curacao", "SA Western Standard Time"),
        ("America/Paramaribo", "SA Eastern Standard Time"),
        ("Etc/UTC", "UTC"),
        ("UTC", "UTC"),
    ];
    ZONES.iter().find(|(i, _)| *i == iana).map(|(_, w)| *w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NS_M, NS_SOAP, NS_T};

    fn reply(items: &str) -> String {
        format!(
            concat!(
                "<s:Envelope xmlns:s=\"{s}\"><s:Body><m:GetItemResponse xmlns:m=\"{m}\" xmlns:t=\"{t}\"><m:ResponseMessages>",
                "<m:GetItemResponseMessage ResponseClass=\"Success\"><m:Items>{items}</m:Items></m:GetItemResponseMessage>",
                "</m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>"
            ),
            s = NS_SOAP,
            m = NS_M,
            t = NS_T,
            items = items
        )
    }

    fn first<T>(xml: &str, of: fn(Node<'_, '_>) -> Option<T>) -> Option<T> {
        let doc = parse(xml).unwrap();
        let items = find(doc.root_element(), "Items").unwrap();
        items.children().filter(|n| n.is_element()).find_map(of)
    }

    #[test]
    fn calendar_items_become_appointments() {
        let xml = reply(concat!(
            "<t:CalendarItem><t:ItemId Id=\"AAk1\" ChangeKey=\"x\"/><t:Subject>Overleg</t:Subject><t:Body BodyType=\"Text\">Agenda:\n- punt 1</t:Body>",
            "<t:ReminderIsSet>true</t:ReminderIsSet><t:ReminderMinutesBeforeStart>30</t:ReminderMinutesBeforeStart>",
            "<t:Start>2026-10-14T17:46:40Z</t:Start><t:End>2026-10-14T18:46:40Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent>",
            "<t:Location>Studio</t:Location></t:CalendarItem>",
        ));
        let a = first(&xml, appointment_of).unwrap();
        assert_eq!((a.id.as_str(), a.title.as_str(), a.location.as_str(), a.notes.as_str()), ("AAk1", "Overleg", "Studio", "Agenda:\n- punt 1"));
        assert_eq!((a.start, a.end, a.all_day, a.reminder), (1_792_000_000, 1_792_003_600, false, Some(30)));
        let off = reply(concat!(
            "<t:CalendarItem><t:ItemId Id=\"A4\"/><t:ReminderIsSet>false</t:ReminderIsSet><t:ReminderMinutesBeforeStart>15</t:ReminderMinutesBeforeStart>",
            "<t:Start>2026-10-14T17:46:40Z</t:Start><t:End>2026-10-14T18:46:40Z</t:End></t:CalendarItem>"
        ));
        assert_eq!(first(&off, appointment_of).unwrap().reminder, None, "minutes without the switch mean no reminder");

        let untitled =
            reply("<t:CalendarItem><t:ItemId Id=\"A2\"/><t:Start>2026-10-14T17:46:40Z</t:Start><t:End>2026-10-14T17:46:40Z</t:End></t:CalendarItem>");
        let a = first(&untitled, appointment_of).unwrap();
        assert_eq!((a.title.as_str(), a.end), ("(Zonder titel)", 1_792_000_001));
        assert!(first(&reply("<t:CalendarItem><t:ItemId Id=\"A3\"/></t:CalendarItem>"), appointment_of).is_none());
    }

    #[test]
    fn contacts_keep_smtp_addresses_only() {
        let xml = reply(concat!(
            "<t:Contact><t:ItemId Id=\"C1\"/><t:Body BodyType=\"Text\">n</t:Body><t:DisplayName>Sanne de Vries</t:DisplayName>",
            "<t:GivenName>Sanne</t:GivenName><t:CompanyName>Studio</t:CompanyName>",
            "<t:EmailAddresses><t:Entry Key=\"EmailAddress1\" RoutingType=\"EX\">/o=Org/cn=Sanne</t:Entry>",
            "<t:Entry Key=\"EmailAddress2\" RoutingType=\"SMTP\">Sanne@Example.org</t:Entry></t:EmailAddresses>",
            "<t:PhoneNumbers><t:Entry Key=\"MobilePhone\">06 1</t:Entry><t:Entry Key=\"HomePhone\"></t:Entry></t:PhoneNumbers>",
            "<t:Surname>de Vries</t:Surname></t:Contact>",
        ));
        let c = first(&xml, contact_of).unwrap();
        assert_eq!((c.name.as_str(), c.company.as_str(), c.notes.as_str()), ("Sanne de Vries", "Studio", "n"));
        assert_eq!(c.emails, vec!["sanne@example.org"]);
        assert_eq!(c.phones, vec!["06 1"]);
        let nameless = reply("<t:Contact><t:ItemId Id=\"C2\"/><t:GivenName>Ravi</t:GivenName><t:Surname>Kumar</t:Surname></t:Contact>");
        assert_eq!(first(&nameless, contact_of).unwrap().name, "Ravi Kumar");
    }

    #[test]
    fn tasks_read_status_importance_and_dates() {
        let xml = reply(concat!(
            "<t:Task><t:ItemId Id=\"T1\"/><t:Subject>Offerte</t:Subject><t:Importance>High</t:Importance>",
            "<t:CompleteDate>2026-10-14T17:46:40Z</t:CompleteDate><t:DueDate>2026-10-13T22:00:00Z</t:DueDate><t:Status>Completed</t:Status></t:Task>",
        ));
        let t = first(&xml, task_of).unwrap();
        assert!(t.done);
        assert_eq!((t.priority, t.due, t.completed), (Priority::High, Some(1_791_928_800), Some(1_792_000_000)));
        let open = reply("<t:Task><t:ItemId Id=\"T2\"/><t:Subject>x</t:Subject><t:Status>InProgress</t:Status></t:Task>");
        let t = first(&open, task_of).unwrap();
        assert!(!t.done && t.due.is_none() && t.priority == Priority::Normal);
    }

    #[test]
    fn create_bodies_follow_the_schema_order_and_escape() {
        let a = Appointment {
            id: String::new(),
            calendar_id: "x".into(),
            title: "A & B".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            all_day: false,
            location: "<Studio>".into(),
            notes: String::new(),
            recurrence: None,
            reminder: Some(15),
        };
        let xml = appointment_fields(&a).create();
        assert_eq!(
            xml,
            concat!(
                "<t:CalendarItem><t:Subject>A &amp; B</t:Subject><t:Body BodyType=\"Text\"></t:Body>",
                "<t:ReminderIsSet>true</t:ReminderIsSet><t:ReminderMinutesBeforeStart>15</t:ReminderMinutesBeforeStart><t:Start>2026-10-14T17:46:40Z</t:Start>",
                "<t:End>2026-10-14T18:46:40Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent><t:Location>&lt;Studio&gt;</t:Location></t:CalendarItem>"
            )
        );
        let whole = Appointment { all_day: true, ..a };
        let xml = appointment_fields(&whole).create();
        // An all-day appointment is sent as local wall-clock time with its offset.
        assert!(xml.contains(&format!("<t:Start>{}</t:Start>", local(1_792_000_000))));
        assert!(!xml.contains("<t:Start>2026-10-14T17:46:40Z"));

        let c = Contact {
            id: String::new(),
            name: "Sanne de Vries".into(),
            emails: vec!["sanne@example.org".into()],
            phones: vec![],
            company: String::new(),
            notes: "n".into(),
            account_id: None,
        };
        let xml = contact_fields(&c).create();
        assert_eq!(
            xml,
            concat!(
                "<t:Contact><t:Body BodyType=\"Text\">n</t:Body><t:GivenName>Sanne</t:GivenName>",
                "<t:EmailAddresses><t:Entry Key=\"EmailAddress1\">sanne@example.org</t:Entry></t:EmailAddresses><t:Surname>de Vries</t:Surname></t:Contact>"
            )
        );
        let (sets, clears) = contact_fields(&c).changes();
        assert!(sets.contains("<t:SetItemField><t:IndexedFieldURI FieldURI=\"contacts:EmailAddress\" FieldIndex=\"EmailAddress1\"/>"));
        assert!(sets.contains("<t:FieldURI FieldURI=\"contacts:Surname\"/><t:Contact><t:Surname>de Vries</t:Surname></t:Contact>"));
        assert!(clears.contains("<t:DeleteItemField><t:FieldURI FieldURI=\"contacts:CompanyName\"/></t:DeleteItemField>"));
        assert!(clears.contains("FieldIndex=\"EmailAddress2\""));
        assert!(clears.contains("FieldIndex=\"BusinessPhone\"") && !sets.contains("CompanyName"));

        let t = Task { title: "Offerte".into(), priority: Priority::Low, done: true, ..Task::default() };
        let xml = task_fields(&t).create();
        assert_eq!(
            xml,
            "<t:Task><t:Subject>Offerte</t:Subject><t:Body BodyType=\"Text\"></t:Body><t:Importance>Low</t:Importance><t:Status>Completed</t:Status></t:Task>"
        );
        let (_, clears) = task_fields(&t).changes();
        assert_eq!(clears, clear("task:DueDate"));
    }

    #[test]
    fn time_zones_map_to_windows_names() {
        assert_eq!(windows_zone("Europe/Amsterdam"), Some("W. Europe Standard Time"));
        assert_eq!(windows_zone("America/New_York"), Some("Eastern Standard Time"));
        assert_eq!(windows_zone("Mars/Olympus"), None);
        assert_eq!(utc(1_792_000_000), "2026-10-14T17:46:40Z");
        let _ = EwsClient::offline(Some("UTC"));
    }
}
