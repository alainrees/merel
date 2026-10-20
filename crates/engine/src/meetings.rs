//! Answers to the meetings the person organises, as Outlook files them: an iTIP `REPLY`
//! (accepted, tentative, declined) or `COUNTER` (a new time proposed) that arrives by mail
//! updates the attendee's answer in the appointment, in the calendar it lives in.
//!
//! An answer counts only when it names an invited attendee and comes from that attendee's own
//! address, for the meeting's current version (`SEQUENCE`), and is newer than the answer
//! already filed. Inbox mail whose subject reads like an answer is fetched on arrival so the
//! appointment is up to date without the message being opened; nothing is ever sent back.

use mailcraft_dav::ical::{self, Component};
use mailcraft_mailbox::Item;
use mailcraft_mime::Attachment;
use mailcraft_model::{Address, Appointment, Attendee, FolderKind, Meeting, Response, parse_address_list};

use crate::Session;

/// Largest calendar part read, in bytes.
const MAX_ICS: usize = 512 * 1024;
/// The most answers fetched ahead per listing.
const MAX_FETCH: usize = 20;

/// The line that older versions of Mailcraft added to the notes of a meeting, before the
/// attendees were kept in the appointment itself.
pub const NOTES_LINE: &str = "Genodigden: ";

/// The subject openings mail programs give an answer, in Dutch and English (Outlook, Gmail,
/// Apple Mail, Thunderbird).
const ANSWER_SUBJECTS: [&str; 12] = [
    "geaccepteerd:",
    "voorlopig:",
    "voorlopig geaccepteerd:",
    "afgewezen:",
    "geweigerd:",
    "nieuwe tijd voorgesteld:",
    "accepted:",
    "tentative:",
    "tentatively accepted:",
    "declined:",
    "new time proposed:",
    "updated invitation with note:",
];

/// What an answer says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub uid: String,
    pub sequence: u32,
    /// When it was sent, its `DTSTAMP`.
    pub stamp: Option<i64>,
    pub attendee: Attendee,
    /// For a `COUNTER`: the start and end proposed.
    pub proposed: Option<(i64, i64)>,
    pub organizer: String,
    pub organizer_name: String,
}

/// Whether `subject` opens the way an answer to an invitation does.
pub fn looks_like_answer(subject: &str) -> bool {
    let s = subject.trim().to_lowercase();
    ANSWER_SUBJECTS.iter().any(|p| s.starts_with(p))
}

fn is_calendar(a: &Attachment) -> bool {
    (a.mime.eq_ignore_ascii_case("text/calendar") || a.mime.eq_ignore_ascii_case("application/ics") || a.name.to_ascii_lowercase().ends_with(".ics"))
        && !a.bytes.is_empty()
        && a.bytes.len() <= MAX_ICS
}

/// The answer in an iCalendar text: a `REPLY` or `COUNTER` with one attendee.
pub fn parse_answer(text: &str) -> Option<Answer> {
    let roots = ical::parse(text);
    let root = roots.iter().find(|c| c.name.eq_ignore_ascii_case("VCALENDAR"))?;
    let method = root.text("METHOD").trim().to_ascii_uppercase();
    if method != "REPLY" && method != "COUNTER" {
        return None;
    }
    let event: &Component = root.find("VEVENT")?;
    let uid = event.text("UID").trim().to_string();
    let attendee = ical::attendee_of(event.prop("ATTENDEE")?);
    if uid.is_empty() || Address::new(None, &attendee.email).is_err() {
        return None;
    }
    let proposed = (method == "COUNTER")
        .then(|| {
            let (start, _) = event.prop("DTSTART").and_then(ical::moment)?;
            let end = event.prop("DTEND").and_then(ical::moment).map(|(e, _)| e).unwrap_or(start + 3_600);
            (end > start).then_some((start, end))
        })
        .flatten();
    let organizer = event.prop("ORGANIZER");
    Some(Answer {
        uid,
        sequence: event.text("SEQUENCE").trim().parse().unwrap_or(0),
        stamp: event.prop("DTSTAMP").and_then(ical::moment).map(|(t, _)| t),
        // A counter proposal leaves the attendee's answer as it was unless it says otherwise.
        attendee: Attendee { replied: None, proposed: None, ..attendee },
        proposed,
        organizer: organizer.map(ical::mailto).unwrap_or_default(),
        organizer_name: organizer.map(ical::common_name).unwrap_or_default(),
    })
}

/// The first answer among the attachments, if any.
pub fn answer_in(attachments: &[Attachment]) -> Option<Answer> {
    attachments.iter().filter(|a| is_calendar(a)).find_map(|a| std::str::from_utf8(&a.bytes).ok().and_then(parse_answer))
}

/// The UID older versions of Mailcraft gave the invitation of `a`, made from its id and start.
fn legacy_uid(a: &Appointment, uid: &str) -> bool {
    let id: String = a.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    uid.starts_with(&format!("{id}-{}@mailcraft.", a.start))
}

/// The attendees named on the notes line older versions wrote, and the notes without it.
pub fn legacy_attendees(notes: &str) -> Option<(Vec<Attendee>, String)> {
    let line = notes.lines().rev().find(|l| l.trim_start().starts_with(NOTES_LINE))?;
    let list = parse_address_list(line.trim_start().trim_start_matches(NOTES_LINE).replace(';', ",").as_str()).ok()?;
    if list.is_empty() {
        return None;
    }
    let attendees = list
        .iter()
        .map(|a| Attendee {
            email: a.email().to_string(),
            name: if a.display() == a.email() { String::new() } else { a.display().to_string() },
            ..Attendee::default()
        })
        .collect();
    let rest: Vec<&str> = notes.lines().filter(|l| l.trim() != line.trim()).collect();
    Some((attendees, rest.join("\n").trim_end().to_string()))
}

impl Session {
    /// Files the answer in `body` of message `message_id`, when it has one; returns the id of
    /// the appointment it changed.
    pub(crate) fn apply_answer_in(&mut self, message_id: &str, attachments: &[Attachment]) -> Option<String> {
        let m = self.message(message_id)?;
        if self.folder(&m.folder_id).is_some_and(|f| f.kind == FolderKind::Junk) {
            return None;
        }
        let from = m.from.email().to_string();
        let answer = answer_in(attachments)?;
        self.apply_answer(&answer, &from)
    }

    /// Files `answer`, sent from `from`, in the meeting it belongs to; returns the id of the
    /// appointment it changed.
    pub fn apply_answer(&mut self, answer: &Answer, from: &str) -> Option<String> {
        if !answer.attendee.email.eq_ignore_ascii_case(from.trim()) {
            return None;
        }
        let index = self.appointments.iter().position(|a| a.meeting.as_ref().is_some_and(|m| m.uid == answer.uid)).or_else(|| {
            self.appointments.iter().position(|a| a.meeting.is_none() && legacy_uid(a, &answer.uid) && legacy_attendees(&a.notes).is_some())
        })?;
        let mut a = self.appointments[index].clone();
        if a.meeting.is_none() {
            // A meeting sent by an older version: its attendees were only on the notes line.
            let (attendees, notes) = legacy_attendees(&a.notes)?;
            a.notes = notes;
            a.meeting = Some(Meeting {
                uid: answer.uid.clone(),
                organizer: answer.organizer.clone(),
                organizer_name: answer.organizer_name.clone(),
                sequence: answer.sequence,
                attendees,
            });
        }
        let meeting = a.meeting.as_mut()?;
        if answer.sequence < meeting.sequence {
            return None;
        }
        let attendee = meeting.attendee_mut(&answer.attendee.email)?;
        if let (Some(old), Some(new)) = (attendee.replied, answer.stamp)
            && new < old
        {
            return None;
        }
        if answer.attendee.response != Response::None || answer.proposed.is_none() {
            attendee.response = answer.attendee.response;
        }
        attendee.proposed = answer.proposed;
        attendee.replied = answer.stamp.or(attendee.replied);
        if attendee.name.is_empty() {
            attendee.name = answer.attendee.name.clone();
        }
        let a = a.validated().ok()?;
        if a == self.appointments[index] {
            return None;
        }
        let id = a.id.clone();
        match self.appointment_account(&a.calendar_id) {
            Some(account) => self.save_remote(&account, false, Item::Appointment(a.clone())).ok()?,
            None => self.calendar_dirty = true,
        }
        self.appointments[index] = a;
        Some(id)
    }

    /// Whether any appointment is a meeting that could get answers.
    fn awaits_answers(&self) -> bool {
        self.appointments.iter().any(|a| a.meeting.as_ref().is_some_and(|m| !m.attendees.is_empty()) || legacy_attendees(&a.notes).is_some())
    }

    /// Asks for the bodies of inbox mail in `folder_id` that reads like an answer, so it is filed
    /// without being opened.
    pub(crate) fn fetch_answers(&mut self, folder_id: &str) {
        if !self.folder(folder_id).is_some_and(|f| f.kind == FolderKind::Inbox) || !self.awaits_answers() {
            return;
        }
        let ids: Vec<String> = self
            .messages
            .iter()
            .filter(|m| m.folder_id == folder_id && looks_like_answer(&m.subject) && !self.bodies.contains_key(&m.id))
            .take(MAX_FETCH)
            .map(|m| m.id.clone())
            .collect();
        for id in ids {
            self.request_body(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;
    use std::sync::Arc;

    fn reply(method: &str, partstat: &str, sequence: u32, stamp: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\nUID:m1@mailcraft.example.org\r\nSEQUENCE:{sequence}\r\nDTSTAMP:{stamp}\r\nDTSTART:20261027T110000Z\r\nDTEND:20261027T120000Z\r\nORGANIZER;CN=Alain:mailto:alain@example.org\r\nATTENDEE;CN=Sanne de Vries;PARTSTAT={partstat}:mailto:sanne@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    fn meeting_session() -> Session {
        let mut s = Engine::sample().session;
        let a = s.appointments.iter_mut().find(|a| a.title == "Overleg ontwerp").expect("sample");
        a.meeting = Some(Meeting {
            uid: "m1@mailcraft.example.org".into(),
            organizer: "alain@example.org".into(),
            organizer_name: "Alain".into(),
            sequence: 0,
            attendees: vec![
                Attendee { email: "sanne@example.org".into(), name: "Sanne".into(), ..Attendee::default() },
                Attendee { email: "ravi@example.org".into(), ..Attendee::default() },
            ],
        });
        s
    }

    fn sanne(s: &Session) -> Attendee {
        let a = s.appointments.iter().find(|a| a.title == "Overleg ontwerp").expect("sample");
        a.meeting.as_ref().expect("meeting").attendees[0].clone()
    }

    #[test]
    fn an_acceptance_is_filed_with_the_attendee() {
        let mut s = meeting_session();
        let answer = parse_answer(&reply("REPLY", "ACCEPTED", 0, "20261021T100000Z")).expect("an answer");
        assert_eq!((answer.attendee.email.as_str(), answer.attendee.response), ("sanne@example.org", Response::Accepted));
        assert!(s.apply_answer(&answer, "ravi@example.org").is_none(), "only Sanne answers for Sanne");
        assert!(s.apply_answer(&answer, "Sanne@Example.org").is_some());
        assert_eq!(sanne(&s).response, Response::Accepted);
        assert!(s.take_calendar_dirty(), "the local calendar is saved");

        // An older answer read later leaves the newer one; a newer one replaces it.
        let older = parse_answer(&reply("REPLY", "DECLINED", 0, "20261020T100000Z")).expect("an answer");
        assert!(s.apply_answer(&older, "sanne@example.org").is_none());
        let newer = parse_answer(&reply("REPLY", "TENTATIVE", 0, "20261022T100000Z")).expect("an answer");
        assert!(s.apply_answer(&newer, "sanne@example.org").is_some());
        assert_eq!(sanne(&s).response, Response::Tentative);
    }

    #[test]
    fn an_answer_to_an_old_version_or_from_a_stranger_changes_nothing() {
        let mut s = meeting_session();
        if let Some(m) = s.appointments.iter_mut().find_map(|a| a.meeting.as_mut()) {
            m.sequence = 2;
        }
        let stale = parse_answer(&reply("REPLY", "ACCEPTED", 1, "20261021T100000Z")).expect("an answer");
        assert!(s.apply_answer(&stale, "sanne@example.org").is_none());
        let stranger = parse_answer(&reply("REPLY", "ACCEPTED", 2, "20261021T100000Z").replace("sanne@", "eve@")).expect("an answer");
        assert!(s.apply_answer(&stranger, "eve@example.org").is_none(), "nobody joins by answering");
        assert_eq!(sanne(&s).response, Response::None);
        assert!(parse_answer(&reply("REQUEST", "ACCEPTED", 0, "20261021T100000Z")).is_none(), "an invitation is no answer");
    }

    #[test]
    fn a_new_time_proposed_is_kept_with_the_attendee() {
        let mut s = meeting_session();
        let counter = parse_answer(&reply("COUNTER", "TENTATIVE", 0, "20261021T100000Z")).expect("an answer");
        assert!(s.apply_answer(&counter, "sanne@example.org").is_some());
        let a = sanne(&s);
        assert_eq!((a.response, a.proposed.map(|(start, end)| end - start)), (Response::Tentative, Some(3_600)));
    }

    #[test]
    fn an_answer_to_a_meeting_sent_by_an_older_version_turns_its_notes_line_into_attendees() {
        let mut s = Engine::sample().session;
        let a = s.appointments.iter_mut().find(|a| a.title == "Lunch met Sanne").expect("sample");
        a.notes = "Tafel reserveren.\n\nGenodigden: Sanne de Vries <sanne@example.org>; ravi@example.org".into();
        let id: String = a.id.chars().filter(char::is_ascii_alphanumeric).collect();
        let uid = format!("{id}-{}@mailcraft.example.org", a.start);
        let text = reply("REPLY", "ACCEPTED", 0, "20261021T100000Z").replace("m1@mailcraft.example.org", &uid);
        let answer = parse_answer(&text).expect("an answer");
        assert!(s.apply_answer(&answer, "sanne@example.org").is_some());
        let a = s.appointments.iter().find(|a| a.title == "Lunch met Sanne").expect("sample");
        assert_eq!(a.notes, "Tafel reserveren.", "the line leaves the notes");
        let m = a.meeting.as_ref().expect("now a meeting");
        assert_eq!((m.uid.as_str(), m.organizer.as_str()), (uid.as_str(), "alain@example.org"));
        let people: Vec<(&str, Response)> = m.attendees.iter().map(|a| (a.email.as_str(), a.response)).collect();
        assert_eq!(people, [("sanne@example.org", Response::Accepted), ("ravi@example.org", Response::None)]);
    }

    #[test]
    fn answers_are_recognised_by_their_subject_and_their_calendar_part() {
        assert!(looks_like_answer("Geaccepteerd: Lunch") && looks_like_answer("Accepted: Lunch @ Tue Oct 27") && looks_like_answer("Afgewezen: x"));
        assert!(!looks_like_answer("Re: Lunch") && !looks_like_answer("Uitnodiging: Lunch"));
        let part = Attachment {
            name: "invite.ics".into(),
            mime: "text/calendar".into(),
            size: 0,
            bytes: Arc::from(reply("REPLY", "DECLINED", 0, "20261021T100000Z").into_bytes()),
        };
        assert_eq!(answer_in(&[part]).map(|a| a.attendee.response), Some(Response::Declined));
    }
}
