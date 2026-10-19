//! iCalendar (RFC 5545) and vCard (RFC 6350) as text: unfolding, properties with parameters,
//! escaping, and the handful of properties Mailcraft understands. Components keep every
//! property they came with, so a change written back leaves alarms, attendees and whatever
//! else the server stored in place.

use std::str::FromStr;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use mailcraft_model::{Appointment, Contact, Priority, Task, clean_text};

/// One `NAME;PARAM=value:value` line, with the value as written (escapes intact).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prop {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub raw: String,
}

impl Prop {
    pub fn new(name: &str, value: &str) -> Prop {
        Prop { name: name.to_string(), params: Vec::new(), raw: escape(value) }
    }

    pub fn with(mut self, param: &str, value: &str) -> Prop {
        self.params.push((param.to_string(), value.to_string()));
        self
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The value with TEXT escapes undone.
    pub fn text(&self) -> String {
        unescape(&self.raw)
    }
}

/// A `BEGIN:NAME` … `END:NAME` block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Component>,
}

impl Component {
    pub fn new(name: &str) -> Component {
        Component { name: name.to_string(), ..Component::default() }
    }

    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn text(&self, name: &str) -> String {
        self.prop(name).map(Prop::text).unwrap_or_default()
    }

    /// Replaces every property called `name` with `prop` (or removes them, for `None`).
    pub fn set(&mut self, name: &str, prop: Option<Prop>) {
        let at = self.props.iter().position(|p| p.name.eq_ignore_ascii_case(name));
        self.props.retain(|p| !p.name.eq_ignore_ascii_case(name));
        if let Some(prop) = prop {
            self.props.insert(at.unwrap_or(self.props.len()), prop);
        }
    }

    pub fn set_text(&mut self, name: &str, value: &str) {
        self.set(name, (!value.is_empty()).then(|| Prop::new(name, value)));
    }

    /// The first component called `name`, at any depth.
    pub fn find(&self, name: &str) -> Option<&Component> {
        self.children.iter().find(|c| c.name.eq_ignore_ascii_case(name)).or_else(|| self.children.iter().find_map(|c| c.find(name)))
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut Component> {
        let at = self.children.iter().position(|c| c.name.eq_ignore_ascii_case(name));
        match at {
            Some(i) => self.children.get_mut(i),
            None => self.children.iter_mut().find_map(|c| c.find_mut(name)),
        }
    }

    /// Every component called `name`, at any depth.
    pub fn all(&self, name: &str) -> Vec<&Component> {
        let mut out = Vec::new();
        for c in &self.children {
            if c.name.eq_ignore_ascii_case(name) {
                out.push(c);
            }
            out.extend(c.all(name));
        }
        out
    }

    /// The text form, lines folded at 75 octets and ended with CRLF.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        fold(out, &format!("BEGIN:{}", self.name));
        for p in &self.props {
            let mut line = p.name.clone();
            for (k, v) in &p.params {
                line.push(';');
                line.push_str(k);
                line.push('=');
                if v.contains([',', ';', ':']) && !v.starts_with('"') {
                    line.push('"');
                    line.push_str(v);
                    line.push('"');
                } else {
                    line.push_str(v);
                }
            }
            line.push(':');
            line.push_str(&p.raw);
            fold(out, &line);
        }
        for c in &self.children {
            c.write(out);
        }
        fold(out, &format!("END:{}", self.name));
    }
}

/// Appends `line` folded so no physical line exceeds 75 octets.
fn fold(out: &mut String, line: &str) {
    let mut bytes = 0;
    let mut first = true;
    for c in line.chars() {
        let n = c.len_utf8();
        if bytes + n > if first { 75 } else { 74 } {
            out.push_str("\r\n ");
            bytes = 0;
            first = false;
        }
        out.push(c);
        bytes += n;
    }
    out.push_str("\r\n");
}

/// TEXT escaping: backslash, comma, semicolon and line breaks.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parses one iCalendar or vCard text into its top-level components; broken input yields
/// what could be read.
pub fn parse(text: &str) -> Vec<Component> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t'))
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
        } else if !raw.is_empty() {
            lines.push(raw.to_string());
        }
    }
    let mut roots = Vec::new();
    let mut stack: Vec<Component> = Vec::new();
    for line in lines {
        let Some(prop) = parse_line(&line) else { continue };
        if prop.name.eq_ignore_ascii_case("BEGIN") {
            if stack.len() < 16 {
                stack.push(Component::new(&prop.raw.trim().to_ascii_uppercase()));
            }
        } else if prop.name.eq_ignore_ascii_case("END") {
            if let Some(done) = stack.pop() {
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => roots.push(done),
                }
            }
        } else if let Some(current) = stack.last_mut() {
            current.props.push(prop);
        }
    }
    roots
}

/// `NAME;PARAM=value;PARAM="quoted:value":value`.
fn parse_line(line: &str) -> Option<Prop> {
    let mut name = String::new();
    let mut params = Vec::new();
    let mut chars = line.char_indices().peekable();
    let mut token = String::new();
    let mut in_quotes = false;
    let mut param_name: Option<String> = None;
    for (i, c) in chars.by_ref() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                match param_name.take() {
                    Some(p) => params.push((p, std::mem::take(&mut token))),
                    None if name.is_empty() => name = std::mem::take(&mut token),
                    None => {}
                }
                let raw = line.get(i + 1..).unwrap_or("").to_string();
                if name.is_empty() {
                    return None;
                }
                return Some(Prop { name: name.to_ascii_uppercase(), params, raw });
            }
            ';' if !in_quotes => match param_name.take() {
                Some(p) => params.push((p, std::mem::take(&mut token))),
                None if name.is_empty() => name = std::mem::take(&mut token),
                None => {}
            },
            '=' if !in_quotes && !name.is_empty() && param_name.is_none() => param_name = Some(std::mem::take(&mut token).to_ascii_uppercase()),
            c => token.push(c),
        }
    }
    None
}

/// The computer's time zone by its IANA name, when the rules are known.
fn local_zone() -> Option<Tz> {
    iana_time_zone::get_timezone().ok().and_then(|name| Tz::from_str(&name).ok())
}

/// A DATE or DATE-TIME property as `(unix seconds, all day)`.
pub fn moment(p: &Prop) -> Option<(i64, bool)> {
    let v = p.raw.trim();
    if p.param("VALUE").is_some_and(|t| t.eq_ignore_ascii_case("DATE")) || (v.len() == 8 && !v.contains('T')) {
        let d = NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        return Some((Local.from_local_datetime(&d.and_time(NaiveTime::MIN)).earliest()?.timestamp(), true));
    }
    if let Some(utc) = v.strip_suffix('Z') {
        let dt = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some((dt.and_utc().timestamp(), false));
    }
    let dt = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    let zone = p.param("TZID").and_then(|z| Tz::from_str(z.trim_matches('/')).ok()).or_else(local_zone);
    let ts = match zone {
        Some(tz) => tz.from_local_datetime(&dt).earliest().map(|d| d.timestamp()),
        None => Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp()),
    }?;
    Some((ts, false))
}

/// `ts` as a UTC DATE-TIME property.
pub fn utc_prop(name: &str, ts: i64) -> Prop {
    let text = DateTime::<Utc>::from_timestamp(ts, 0).map(|d| d.format("%Y%m%dT%H%M%SZ").to_string()).unwrap_or_else(|| "19700101T000000Z".into());
    Prop { name: name.to_string(), params: Vec::new(), raw: text }
}

/// The local day of `ts` as a DATE property.
pub fn date_prop(name: &str, ts: i64) -> Prop {
    let text = Local.timestamp_opt(ts, 0).single().map(|d| d.format("%Y%m%d").to_string()).unwrap_or_else(|| "19700101".into());
    Prop { name: name.to_string(), params: vec![("VALUE".into(), "DATE".into())], raw: text }
}

fn title_or(text: String) -> String {
    let t = clean_text(text.trim());
    if t.is_empty() { "(Zonder titel)".into() } else { t }
}

/// The appointment in a VEVENT; `None` for one without a usable start.
pub fn appointment_of(event: &Component, id: String) -> Option<Appointment> {
    if event.text("STATUS").eq_ignore_ascii_case("CANCELLED") {
        return None;
    }
    let (start, all_day) = event.prop("DTSTART").and_then(moment)?;
    let end = match event.prop("DTEND").and_then(moment) {
        Some((end, _)) => end,
        None => match event.prop("DURATION").map(|p| duration_seconds(&p.raw)) {
            Some(Some(d)) => start + d,
            _ if all_day => start + 86_400,
            _ => start + 3_600,
        },
    };
    Appointment {
        id,
        calendar_id: String::new(),
        title: title_or(event.text("SUMMARY")),
        start,
        end: end.max(start + 1),
        all_day,
        location: event.text("LOCATION"),
        notes: event.text("DESCRIPTION"),
        recurrence: None,
        reminder: reminder_of(event),
    }
    .validated()
    .ok()
}

/// The first alarm that fires before the start, in minutes: a `VALARM` whose `TRIGGER` is a
/// (negative) duration from the start. Alarms at an absolute time or relative to the end are
/// not Mailcraft's reminder and are left alone.
fn reminder_of(event: &Component) -> Option<u32> {
    event.children.iter().filter(|c| c.name.eq_ignore_ascii_case("VALARM")).find_map(|alarm| {
        let trigger = alarm.prop("TRIGGER")?;
        if trigger.param("RELATED").is_some_and(|r| r.eq_ignore_ascii_case("END"))
            || trigger.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"))
        {
            return None;
        }
        let seconds = duration_seconds(&trigger.raw)?;
        u32::try_from((-seconds).max(0) / 60).ok()
    })
}

/// Replaces the alarms that fire before the start with one for `minutes` before it (or with
/// none); alarms at an absolute time or relative to the end stay.
fn write_reminder(event: &mut Component, title: &str, minutes: Option<u32>) {
    event.children.retain(|c| {
        if !c.name.eq_ignore_ascii_case("VALARM") {
            return true;
        }
        let Some(trigger) = c.prop("TRIGGER") else { return false };
        trigger.param("RELATED").is_some_and(|r| r.eq_ignore_ascii_case("END"))
            || trigger.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"))
    });
    if let Some(m) = minutes {
        let mut alarm = Component::new("VALARM");
        alarm.props.push(Prop::new("ACTION", "DISPLAY"));
        alarm.props.push(Prop::new("DESCRIPTION", title));
        alarm.props.push(Prop { name: "TRIGGER".into(), params: Vec::new(), raw: if m == 0 { "PT0S".into() } else { format!("-PT{m}M") } });
        event.children.push(alarm);
    }
}

/// `PT1H30M`, `P1D` and the like, in seconds.
fn duration_seconds(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let rest = rest.strip_prefix('P')?;
    let mut total = 0i64;
    let mut number = String::new();
    let mut in_time = false;
    for c in rest.chars() {
        match c {
            '0'..='9' => number.push(c),
            'T' => in_time = true,
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                total += n * match (unit, in_time) {
                    ('W', _) => 7 * 86_400,
                    ('D', _) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    Some(sign * total)
}

/// `ts` as a DATE-TIME property in the computer's time zone (`TZID=Europe/Amsterdam`), so a
/// series keeps its wall-clock time across a daylight-saving change; UTC when the zone is not
/// known.
pub fn zoned_prop(name: &str, ts: i64) -> Prop {
    match local_zone().and_then(|tz| tz.timestamp_opt(ts, 0).single().map(|d| (tz.name().to_string(), d.format("%Y%m%dT%H%M%S").to_string()))) {
        Some((zone, text)) => Prop { name: name.to_string(), params: vec![("TZID".into(), zone)], raw: text },
        None => utc_prop(name, ts),
    }
}

/// Writes `a` into `event`, keeping what else it holds. A series gets its `RRULE` and, when
/// timed, a start and end in the computer's time zone rather than UTC.
pub fn write_appointment(event: &mut Component, a: &Appointment) {
    event.set_text("SUMMARY", &a.title);
    event.set_text("LOCATION", &a.location);
    event.set_text("DESCRIPTION", &a.notes);
    event.set("DURATION", None);
    if a.all_day {
        event.set("DTSTART", Some(date_prop("DTSTART", a.start)));
        event.set("DTEND", Some(date_prop("DTEND", a.end.max(a.start + 86_400))));
    } else if a.recurrence.is_some() {
        event.set("DTSTART", Some(zoned_prop("DTSTART", a.start)));
        event.set("DTEND", Some(zoned_prop("DTEND", a.end)));
    } else {
        event.set("DTSTART", Some(utc_prop("DTSTART", a.start)));
        event.set("DTEND", Some(utc_prop("DTEND", a.end)));
    }
    if let Some(r) = &a.recurrence {
        event.set("RRULE", Some(Prop { name: "RRULE".into(), params: Vec::new(), raw: r.rrule(a.all_day) }));
    }
    write_reminder(event, &a.title, a.reminder);
    stamp(event);
}

/// The task in a VTODO.
pub fn task_of(todo: &Component, id: String) -> Option<Task> {
    let done = todo.text("STATUS").eq_ignore_ascii_case("COMPLETED") || todo.text("PERCENT-COMPLETE").trim() == "100";
    let priority = match todo.text("PRIORITY").trim().parse::<u8>().unwrap_or(0) {
        1..=4 => Priority::High,
        6..=9 => Priority::Low,
        _ => Priority::Normal,
    };
    Task {
        id,
        title: title_or(todo.text("SUMMARY")),
        notes: todo.text("DESCRIPTION"),
        due: todo.prop("DUE").and_then(moment).map(|(ts, _)| ts),
        priority,
        done,
        completed: todo.prop("COMPLETED").and_then(moment).map(|(ts, _)| ts),
        account_id: None,
    }
    .validated()
    .ok()
}

pub fn write_task(todo: &mut Component, t: &Task) {
    todo.set_text("SUMMARY", &t.title);
    todo.set_text("DESCRIPTION", &t.notes);
    todo.set("DUE", t.due.map(|d| date_prop("DUE", d)));
    let priority = match t.priority {
        Priority::High => Some("1"),
        Priority::Normal => None,
        Priority::Low => Some("9"),
    };
    todo.set("PRIORITY", priority.map(|p| Prop::new("PRIORITY", p)));
    if t.done {
        todo.set("STATUS", Some(Prop::new("STATUS", "COMPLETED")));
        todo.set("PERCENT-COMPLETE", Some(Prop::new("PERCENT-COMPLETE", "100")));
        todo.set("COMPLETED", Some(utc_prop("COMPLETED", t.completed.unwrap_or_else(|| Utc::now().timestamp()))));
    } else {
        todo.set("STATUS", Some(Prop::new("STATUS", "NEEDS-ACTION")));
        todo.set("PERCENT-COMPLETE", None);
        todo.set("COMPLETED", None);
    }
    stamp(todo);
}

/// The contact in a VCARD.
pub fn contact_of(card: &Component, id: String) -> Option<Contact> {
    let mut name = card.text("FN").trim().to_string();
    if name.is_empty() {
        let n = card.text("N");
        let mut parts = n.split(';');
        let family = parts.next().unwrap_or("").trim();
        let given = parts.next().unwrap_or("").trim();
        name = format!("{given} {family}").trim().to_string();
    }
    let values = |prop: &str| -> Vec<String> {
        card.props.iter().filter(|p| p.name.eq_ignore_ascii_case(prop)).map(|p| p.text().trim().to_string()).filter(|v| !v.is_empty()).collect()
    };
    Contact {
        id,
        name,
        emails: values("EMAIL").into_iter().filter(|e| mailcraft_model::Address::new(None, e).is_ok()).collect(),
        phones: values("TEL").into_iter().map(|t| t.trim_start_matches("tel:").to_string()).collect(),
        company: card.text("ORG").split(';').next().unwrap_or("").trim().to_string(),
        notes: card.text("NOTE"),
        account_id: None,
    }
    .validated()
    .ok()
}

pub fn write_contact(card: &mut Component, c: &Contact) {
    card.set_text("FN", &c.name);
    let (given, family) = c.name.split_once(' ').unwrap_or((c.name.as_str(), ""));
    card.set("N", Some(Prop { name: "N".into(), params: Vec::new(), raw: format!("{};{};;;", escape(family.trim()), escape(given.trim())) }));
    card.set("ORG", (!c.company.is_empty()).then(|| Prop::new("ORG", &c.company)));
    card.set_text("NOTE", &c.notes);
    card.set("EMAIL", None);
    card.set("TEL", None);
    for e in &c.emails {
        card.props.push(Prop::new("EMAIL", e));
    }
    for p in &c.phones {
        card.props.push(Prop::new("TEL", p));
    }
    if card.prop("VERSION").is_none() {
        card.props.insert(0, Prop::new("VERSION", "3.0"));
    }
    card.set("REV", Some(utc_prop("REV", Utc::now().timestamp())));
}

/// A fresh VCALENDAR holding one component of `kind` (`VEVENT` or `VTODO`) with `uid`.
pub fn new_calendar(kind: &str, uid: &str) -> Component {
    let mut root = Component::new("VCALENDAR");
    root.props.push(Prop::new("VERSION", "2.0"));
    root.props.push(Prop::new("PRODID", "-//ArtCraft//Mailcraft//NL"));
    let mut inner = Component::new(kind);
    inner.props.push(Prop::new("UID", uid));
    inner.props.push(utc_prop("DTSTAMP", Utc::now().timestamp()));
    root.children.push(inner);
    root
}

pub fn new_card(uid: &str) -> Component {
    let mut card = Component::new("VCARD");
    card.props.push(Prop::new("VERSION", "3.0"));
    card.props.push(Prop::new("UID", uid));
    card
}

/// Refreshes DTSTAMP and LAST-MODIFIED, as a change should.
fn stamp(c: &mut Component) {
    let now = Utc::now().timestamp();
    c.set("DTSTAMP", Some(utc_prop("DTSTAMP", now)));
    c.set("LAST-MODIFIED", Some(utc_prop("LAST-MODIFIED", now)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENT: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Amsterdam\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:abc@x\r\nDTSTAMP:20261001T120000Z\r\nDTSTART;TZID=Europe/Amsterdam:20261019T100000\r\nDTEND;TZID=Europe/Amsterdam:20261019T113000\r\nSUMMARY:Lunch met Sanne\\, Jan\r\nLOCATION:Kantine\r\nDESCRIPTION:Regel eerst\\n de offerte\r\n  en de rest\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn events_are_read_with_their_time_zone_and_written_back_whole() {
        let roots = parse(EVENT);
        let event = roots[0].find("VEVENT").unwrap();
        let a = appointment_of(event, "e:1".into()).unwrap();
        // 10:00 Amsterdam in October (CEST, UTC+2) is 08:00 UTC.
        assert_eq!((a.start, a.end), (1_792_396_800, 1_792_402_200));
        assert_eq!(
            (a.title.as_str(), a.location.as_str(), a.notes.as_str()),
            ("Lunch met Sanne, Jan", "Kantine", "Regel eerst\n de offerte en de rest")
        );
        assert!(!a.all_day);
        assert_eq!(a.reminder, Some(15), "the alarm 15 minutes before is the reminder");

        let mut root = roots[0].clone();
        let changed = Appointment { title: "Lunch".into(), all_day: true, end: a.start + 86_400 * 2, reminder: Some(60), ..a.clone() };
        write_appointment(root.find_mut("VEVENT").unwrap(), &changed);
        let text = root.serialize();
        assert!(
            text.contains("BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Lunch\r\nTRIGGER:-PT60M\r\nEND:VALARM"),
            "the alarm is rewritten: {text}"
        );
        assert_eq!(text.matches("BEGIN:VALARM").count(), 1);
        let mut silent = root.clone();
        write_appointment(silent.find_mut("VEVENT").unwrap(), &Appointment { reminder: None, ..changed.clone() });
        assert!(!silent.serialize().contains("VALARM"), "no reminder, no alarm");
        assert!(text.contains("DTSTART;VALUE=DATE:2026"), "{text}");
        assert!(!text.contains("TZID=Europe/Amsterdam:2026"), "{text}");
        assert!(text.contains("SUMMARY:Lunch\r\n"));
        let again = parse(&text);
        let b = appointment_of(again[0].find("VEVENT").unwrap(), "e:1".into()).unwrap();
        assert!(b.all_day && b.end - b.start == 86_400 * 2);

        // A series carries its rule, with a start in a named zone when the computer has one.
        let weekly = mailcraft_model::Recurrence {
            interval: 2,
            weekdays: vec![0, 2],
            count: Some(10),
            ..mailcraft_model::Recurrence::every(mailcraft_model::Frequency::Weekly)
        };
        let series = Appointment { all_day: false, recurrence: Some(weekly), ..a.clone() };
        let mut root = parse(EVENT).remove(0);
        write_appointment(root.find_mut("VEVENT").unwrap(), &series);
        let text = root.serialize();
        assert!(text.contains("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10\r\n"), "{text}");
        let start = parse(&text)[0].find("VEVENT").unwrap().prop("DTSTART").cloned().unwrap();
        assert!(start.param("TZID").is_some() || start.raw.ends_with('Z'), "{start:?}");
        assert_eq!(moment(&start).map(|m| m.0), Some(series.start), "the zoned start names the same moment");
    }

    #[test]
    fn utc_floating_all_day_and_duration_forms_parse() {
        let p = |line: &str| parse_line(line).unwrap();
        assert_eq!(moment(&p("DTSTART:20261019T080000Z")), Some((1_792_396_800, false)));
        assert_eq!(moment(&p("DTSTART;VALUE=DATE:20261019")).map(|m| m.1), Some(true));
        assert_eq!(moment(&p("DTSTART:20261019")).map(|m| m.1), Some(true));
        assert!(moment(&p("DTSTART:20261019T080000")).is_some(), "floating time is local time");
        assert!(moment(&p("DTSTART;TZID=W. Europe Standard Time:20261019T080000")).is_some(), "an unknown zone falls back to local time");
        assert_eq!(duration_seconds("PT1H30M"), Some(5_400));
        assert_eq!(duration_seconds("P1DT2H"), Some(93_600));
        assert_eq!(duration_seconds("-P1W"), Some(-7 * 86_400));
        assert_eq!(duration_seconds("1H"), None);
        let roots =
            parse("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20261019T080000Z\r\nDURATION:PT2H\r\nSUMMARY:x\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
        let a = appointment_of(roots[0].find("VEVENT").unwrap(), "e".into()).unwrap();
        assert_eq!(a.end - a.start, 7_200);
    }

    #[test]
    fn lines_fold_unfold_and_escape() {
        let long = "x".repeat(200);
        let mut c = Component::new("VEVENT");
        c.props.push(Prop::new("SUMMARY", &format!("a;b,c\\d\nнет {long}")));
        c.props.push(Prop::new("X-Q", "v").with("TZID", "America/New_York").with("CN", "Doe, John"));
        let text = c.serialize();
        assert!(text.lines().all(|l| l.len() <= 75), "{text}");
        assert!(text.contains("X-Q;TZID=America/New_York;CN=\"Doe, John\":v"));
        let back = parse(&format!("BEGIN:VCALENDAR\r\n{text}END:VCALENDAR\r\n"));
        let event = back[0].find("VEVENT").unwrap();
        assert_eq!(event.text("SUMMARY"), format!("a;b,c\\d\nнет {long}"));
        assert_eq!(event.prop("X-Q").unwrap().param("cn"), Some("Doe, John"));
        assert_eq!(event.prop("X-Q").unwrap().param("TZID"), Some("America/New_York"));
        assert!(parse("garbage\r\nEND:VEVENT\r\n").is_empty());
    }

    #[test]
    fn tasks_and_cards_round_trip() {
        let todo_text = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Offerte\r\nDUE;VALUE=DATE:20261020\r\nPRIORITY:2\r\nSTATUS:NEEDS-ACTION\r\nX-KEEP:1\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let mut root = parse(todo_text).remove(0);
        let t = task_of(root.find("VTODO").unwrap(), "t:1".into()).unwrap();
        assert_eq!((t.title.as_str(), t.priority, t.done), ("Offerte", Priority::High, false));
        assert!(t.due.is_some());
        let done = Task { done: true, completed: Some(1_792_396_800), priority: Priority::Low, due: None, ..t.clone() };
        write_task(root.find_mut("VTODO").unwrap(), &done);
        let text = root.serialize();
        assert!(text.contains("STATUS:COMPLETED") && text.contains("PRIORITY:9") && text.contains("X-KEEP:1") && !text.contains("DUE"), "{text}");
        let t2 = task_of(parse(&text)[0].find("VTODO").unwrap(), "t:1".into()).unwrap();
        assert!(t2.done && t2.completed == Some(1_792_396_800) && t2.priority == Priority::Low && t2.due.is_none());

        let card_text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:c1\r\nN:de Vries;Sanne;;;\r\nFN:Sanne de Vries\r\nEMAIL;TYPE=INTERNET:sanne@example.org\r\nEMAIL:not an address\r\nTEL;TYPE=CELL:+31 6 1234\r\nORG:Bakkerij Jansen;Verkoop\r\nNOTE:Klant\r\nPHOTO;VALUE=URI:https://x/y.png\r\nEND:VCARD\r\n";
        let mut card = parse(card_text).remove(0);
        let c = contact_of(&card, "c:1".into()).unwrap();
        assert_eq!((c.name.as_str(), c.company.as_str(), c.notes.as_str()), ("Sanne de Vries", "Bakkerij Jansen", "Klant"));
        assert_eq!((c.emails.clone(), c.phones.clone()), (vec!["sanne@example.org".to_string()], vec!["+31 6 1234".to_string()]));
        let changed = Contact {
            name: "Sanne Jansen".into(),
            emails: vec!["s@j.nl".into(), "sanne@example.org".into()],
            phones: vec![],
            company: String::new(),
            ..c.clone()
        };
        write_contact(&mut card, &changed);
        let text = card.serialize();
        assert!(
            text.contains("PHOTO;VALUE=URI:https://x/y.png") && text.contains("N:Jansen;Sanne;;;") && !text.contains("ORG") && !text.contains("TEL"),
            "{text}"
        );
        let c2 = contact_of(&parse(&text)[0], "c:1".into()).unwrap();
        assert_eq!(c2.emails, vec!["s@j.nl".to_string(), "sanne@example.org".to_string()]);
        let only_n = parse("BEGIN:VCARD\r\nN:Jansen;Piet;;;\r\nEND:VCARD\r\n").remove(0);
        assert_eq!(contact_of(&only_n, "c".into()).unwrap().name, "Piet Jansen");
    }

    #[test]
    fn new_components_carry_what_a_server_requires() {
        let cal = new_calendar("VEVENT", "u1@mailcraft");
        let text = cal.serialize();
        assert!(text.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:") && text.contains("UID:u1@mailcraft") && text.contains("DTSTAMP:"));
        let card = new_card("u2@mailcraft").serialize();
        assert!(card.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u2@mailcraft\r\n"));
    }
}
