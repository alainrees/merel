//! Meeting requests, as Outlook's "Uitnodigen": the people under Genodigden in the appointment
//! window get a message with the appointment as a `text/calendar` request, sent from the account
//! the calendar belongs to. The appointment keeps them as its meeting, with their answers; a
//! change that matters goes out as an update ("Update verzenden"), people taken off and a
//! meeting called off get a cancellation. Nothing goes out without the person pressing the
//! button that says so.

use mailcraft_dav::ical::{self, Prop};
use mailcraft_engine::{SAMPLE_ACCOUNT, Session};
use mailcraft_model::{Account, Address, Appointment, Attendee, Draft, DraftAttachment, Meeting, Response, parse_address_list};

use crate::compose::{Compose, Kind};
use crate::invitation;

pub const FIELD: &str = "Genodigden";
pub const OPTIONAL: &str = "Optioneel";
pub const FILE: &str = "uitnodiging.ics";
pub const MIME: &str = "text/calendar; method=REQUEST; charset=utf-8";
pub const CANCEL_FILE: &str = "annulering.ics";
pub const CANCEL_MIME: &str = "text/calendar; method=CANCEL; charset=utf-8";
pub const NO_ACCOUNT: &str = "Voeg eerst een echt account toe om een uitnodiging te kunnen versturen; de afspraak is nog niet opgeslagen.";
pub const BAD_ADDRESSES: &str = "Controleer de adressen bij Genodigden.";
/// The button in the window of a saved appointment that passes it on by mail.
pub const FORWARD: &str = "Doorsturen…";
pub const FORWARD_FILE: &str = "afspraak.ics";
pub const FORWARD_MIME: &str = "text/calendar; method=PUBLISH; charset=utf-8";
/// The main button of a meeting that changed since it went out.
pub const SEND_UPDATE: &str = "Update verzenden";
/// The delete button of a meeting the person organises.
pub const CANCEL_MEETING: &str = "Vergadering annuleren";

/// The addresses typed under Genodigden, or what is wrong with them; empty for no invitation.
pub fn attendees(text: &str) -> Result<Vec<Address>, String> {
    parse_address_list(text).map_err(|_| BAD_ADDRESSES.to_string())
}

/// An address as an attendee who has not answered yet.
pub fn attendee(a: &Address, optional: bool) -> Attendee {
    let name = if a.display() == a.email() { String::new() } else { a.display().to_string() };
    Attendee { email: a.email().to_string(), name, optional, ..Attendee::default() }
}

/// The account an invitation for `calendar_id` goes out from: the calendar's own account, else
/// the account of the open folder. `None` when there is only the sample.
pub fn account_for(session: &Session, calendar_id: Option<&str>) -> Option<Account> {
    let own = calendar_id.and_then(|c| session.appointment_account(c)).and_then(|id| session.accounts.iter().find(|a| a.id == id));
    own.or_else(|| session.current_account()).filter(|a| a.id != SAMPLE_ACCOUNT).cloned()
}

/// Whether one of the person's own accounts organises `m`.
pub fn organised_here(session: &Session, m: &Meeting) -> bool {
    m.organizer.is_empty() || session.accounts.iter().any(|a| m.organized_by(a.address.email()))
}

/// A fresh UID for a meeting organised from `organizer`.
pub fn new_uid(organizer: &Address) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let domain = organizer.email().rsplit_once('@').map(|(_, d)| d).unwrap_or("local");
    format!("{nanos:x}@mailcraft.{domain}")
}

/// The UID older versions gave the invitation of `a`: its id and start, at the organiser's domain.
pub fn legacy_uid(a: &Appointment, organizer: &Address) -> String {
    let id: String = a.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let domain = organizer.email().rsplit_once('@').map(|(_, d)| d).unwrap_or("local");
    format!("{id}-{}@mailcraft.{domain}", a.start)
}

/// The organiser of `m` as an address, for the text of the message.
fn organizer_of(m: &Meeting) -> Option<Address> {
    Address::new((!m.organizer_name.is_empty()).then_some(m.organizer_name.as_str()), &m.organizer).ok()
}

fn address_of(a: &Attendee) -> Option<Address> {
    Address::new((!a.name.is_empty()).then_some(a.name.as_str()), &a.email).ok()
}

/// The iCalendar message for `a`'s meeting with `method` (`REQUEST` or `CANCEL`), naming
/// `attendees`, so mail programs show it as an invitation or a cancellation.
pub fn ics(a: &Appointment, method: &str, attendees: &[Attendee]) -> String {
    let m = a.meeting.clone().unwrap_or_default();
    let mut root = ical::new_calendar("VEVENT", &m.uid);
    root.props.push(Prop::new("METHOD", method));
    if let Some(event) = root.find_mut("VEVENT") {
        ical::write_appointment(event, a);
        // What travels by mail names the people without the server's scheduling hint.
        event.props.retain(|p| !p.name.eq_ignore_ascii_case("ATTENDEE") && !p.name.eq_ignore_ascii_case("X-MAILCRAFT-UID"));
        event.set("STATUS", Some(Prop::new("STATUS", if method == "CANCEL" { "CANCELLED" } else { "CONFIRMED" })));
        event.set("SEQUENCE", Some(Prop::new("SEQUENCE", &m.sequence.to_string())));
        event.set("ORGANIZER", Some(ical::organizer_prop(&m)));
        for p in attendees {
            event.props.push(ical::attendee_prop(p));
        }
    }
    ical::add_timezones(&mut root);
    root.serialize()
}

/// The text of the message, for mail programs that do not read the calendar part.
fn words(a: &Appointment, opening: &str, closing: &str) -> String {
    let mut body = format!("{opening}{}\nWanneer: {}\n", a.title.trim(), invitation::when(a));
    if !a.location.trim().is_empty() {
        body.push_str(&format!("Waar: {}\n", a.location.trim()));
    }
    if let Some(o) = a.meeting.as_ref().and_then(organizer_of) {
        body.push_str(&format!("Organisator: {}\n", mailbox(&o)));
    }
    if !a.notes.trim().is_empty() {
        body.push_str(&format!("\n{}\n", a.notes.trim()));
    }
    body.push_str(closing);
    body
}

/// The request to every attendee of `a`'s meeting: the appointment in words and the request
/// itself as the attachment; `update` for a meeting that went out before. `None` without
/// attendees.
pub fn request(a: &Appointment, update: bool) -> Option<Draft> {
    let people = &a.meeting.as_ref()?.attendees;
    let to: Vec<Address> = people.iter().filter_map(address_of).collect();
    if to.is_empty() {
        return None;
    }
    let (opening, subject) = if update { ("De afspraak is gewijzigd: ", "Bijgewerkt") } else { ("Je bent uitgenodigd voor: ", "Uitnodiging") };
    let body = words(a, opening, "\nDe bijgevoegde uitnodiging zet de afspraak in je agenda.\n");
    let mut attachment = DraftAttachment::new(FILE, ics(a, "REQUEST", people).into_bytes());
    attachment.mime = MIME.to_string();
    Some(Draft { to, subject: format!("{subject}: {}", a.title.trim()), body, attachments: vec![attachment], ..Draft::default() })
}

/// The cancellation to `people` (all attendees when the meeting is called off, or those taken
/// off it). `None` for nobody.
pub fn cancellation(a: &Appointment, people: &[Attendee]) -> Option<Draft> {
    let to: Vec<Address> = people.iter().filter_map(address_of).collect();
    if to.is_empty() || a.meeting.is_none() {
        return None;
    }
    let body = words(a, "Deze afspraak gaat niet door: ", "\nMet de bijgevoegde annulering haal je hem uit je agenda.\n");
    let mut attachment = DraftAttachment::new(CANCEL_FILE, ics(a, "CANCEL", people).into_bytes());
    attachment.mime = CANCEL_MIME.to_string();
    Some(Draft { to, subject: format!("Geannuleerd: {}", a.title.trim()), body, attachments: vec![attachment], ..Draft::default() })
}

/// "Geaccepteerd", "Voorlopig", … for an answer.
pub fn response_label(r: Response) -> &'static str {
    match r {
        Response::Accepted => "Geaccepteerd",
        Response::Tentative => "Voorlopig",
        Response::Declined => "Afgewezen",
        Response::None => "Nog geen reactie",
    }
}

/// The mark before an attendee's name: ✔, ?, ✕, or nothing yet.
pub fn response_mark(r: Response) -> &'static str {
    match r {
        Response::Accepted => "✔",
        Response::Tentative => "?",
        Response::Declined => "✕",
        Response::None => "",
    }
}

/// "1 geaccepteerd, 1 nog geen reactie": who answered what, as Outlook's tracking line.
pub fn tally_line(m: &Meeting) -> String {
    let [yes, maybe, no, none] = m.tally();
    let mut parts = Vec::new();
    for (n, what) in [(yes, "geaccepteerd"), (maybe, "voorlopig"), (no, "afgewezen"), (none, "nog geen reactie")] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    parts.join(", ")
}

/// The appointment as a published calendar file, for passing it on: no organiser, no attendees
/// and nothing to answer, so the reader's program simply offers to add it.
pub fn published(a: &Appointment) -> String {
    let id: String = a.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut root = ical::new_calendar("VEVENT", &format!("{id}-{}@mailcraft.local", a.start));
    root.props.push(Prop::new("METHOD", "PUBLISH"));
    if let Some(event) = root.find_mut("VEVENT") {
        ical::write_appointment(event, &Appointment { meeting: None, ..a.clone() });
    }
    ical::add_timezones(&mut root);
    root.serialize()
}

/// The message that passes an appointment on, Outlook's "Doorsturen" on an appointment: the
/// appointment in words, and as an attachment the reader can put in their own calendar. The
/// recipients are left to the person.
pub fn forward(a: &Appointment) -> Draft {
    let mut body = format!("Afspraak: {}\nWanneer: {}\n", a.title.trim(), invitation::when(a));
    if !a.location.trim().is_empty() {
        body.push_str(&format!("Waar: {}\n", a.location.trim()));
    }
    if !a.notes.trim().is_empty() {
        body.push_str(&format!("\n{}\n", a.notes.trim()));
    }
    body.push_str("\nMet de bijgevoegde afspraak zet je hem in je eigen agenda.\n");
    let mut attachment = DraftAttachment::new(FORWARD_FILE, published(a).into_bytes());
    attachment.mime = FORWARD_MIME.to_string();
    Draft { subject: format!("Doorgestuurd: {}", a.title.trim()), body, attachments: vec![attachment], ..Draft::default() }
}

/// The compose window for passing `a` on, from the calendar's account (else the current one),
/// with that account's signature under the text.
pub fn forward_window(session: &Session, a: &Appointment) -> Compose {
    let account =
        session.appointment_account(&a.calendar_id).and_then(|id| session.accounts.iter().find(|x| x.id == id)).or_else(|| session.current_account());
    let (account_id, signature) = account.map(|x| (x.id.clone(), x.signature.clone())).unwrap_or_default();
    let d = forward(a);
    let mut c = Compose::new(Kind::New, None).with_signature(&signature).opened_from(&account_id);
    c.subject = d.subject;
    c.body = format!("{}{}", d.body, c.body);
    c.attachments = d.attachments;
    c
}

fn mailbox(a: &Address) -> String {
    if a.display() == a.email() { a.email().to_string() } else { format!("{} <{}>", a.display(), a.email()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appointment() -> Appointment {
        Appointment {
            id: "a/7".into(),
            calendar_id: "local".into(),
            title: "Overleg ontwerp".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            all_day: false,
            location: "Kamer 2".into(),
            notes: "Schetsen meenemen.".into(),
            notes_rich: None,
            recurrence: None,
            reminder: Some(15),
            private: false,
            show_as: mailcraft_model::ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
        }
    }

    fn meeting() -> Appointment {
        let me = Address::new(Some("Alain"), "alain@example.org").unwrap();
        let sanne = Address::new(Some("Sanne de Vries"), "sanne@example.org").unwrap();
        let ravi = Address::new(None, "ravi@example.org").unwrap();
        let mut a = appointment();
        a.meeting = Some(Meeting {
            uid: legacy_uid(&a, &me),
            organizer: me.email().into(),
            organizer_name: "Alain".into(),
            sequence: 0,
            attendees: vec![attendee(&sanne, false), attendee(&ravi, true)],
        });
        a
    }

    #[test]
    fn the_request_names_the_organiser_the_attendees_and_the_method() {
        let a = meeting();
        let people = a.meeting.clone().unwrap().attendees;
        let text = ics(&a, "REQUEST", &people).replace("\r\n ", "");
        assert!(text.contains("METHOD:REQUEST\r\n"), "{text}");
        assert!(text.contains("UID:a7-1792000000@mailcraft.example.org\r\n"), "{text}");
        assert!(text.contains("ORGANIZER;CN=Alain:mailto:alain@example.org\r\n"), "{text}");
        assert!(
            text.contains("ATTENDEE;CN=Sanne de Vries;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:sanne@example.org\r\n"),
            "{text}"
        );
        assert!(text.contains("ATTENDEE;ROLE=OPT-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:ravi@example.org\r\n"), "{text}");
        assert!(text.contains("SUMMARY:Overleg ontwerp\r\n") && text.contains("LOCATION:Kamer 2\r\n") && text.contains("STATUS:CONFIRMED\r\n"));
        assert!(!text.contains("SCHEDULE-AGENT"), "the hint is for servers, not for mail: {text}");
        // Our own reading of invitations understands it.
        let inv = invitation::parse(&text).expect("an invitation");
        assert_eq!((inv.title.as_str(), inv.organizer.as_str(), inv.cancelled), ("Overleg ontwerp", "Alain", false));
        assert_eq!(inv.appointment.map(|a| (a.start, a.end, a.location)), Some((1_792_000_000, 1_792_003_600, "Kamer 2".to_string())));

        let d = request(&a, false).expect("a request");
        assert_eq!((d.subject.as_str(), d.to.len(), d.attachments.len()), ("Uitnodiging: Overleg ontwerp", 2, 1));
        assert_eq!((d.attachments[0].name.as_str(), d.attachments[0].mime.as_str()), (FILE, MIME));
        assert!(d.body.starts_with("Je bent uitgenodigd voor: Overleg ontwerp\nWanneer: ") && d.body.contains("Waar: Kamer 2\n"), "{}", d.body);
        assert!(d.body.contains("Organisator: Alain <alain@example.org>\n") && d.body.contains("\nSchetsen meenemen.\n"));
        assert_eq!(request(&a, true).map(|d| d.subject), Some("Bijgewerkt: Overleg ontwerp".to_string()));
        assert!(request(&appointment(), false).is_none(), "an appointment without attendees invites nobody");
    }

    #[test]
    fn a_cancellation_goes_to_the_people_named() {
        let a = meeting();
        let ravi = a.meeting.clone().unwrap().attendees[1].clone();
        let d = cancellation(&a, std::slice::from_ref(&ravi)).expect("a cancellation");
        assert_eq!((d.subject.as_str(), d.to.len()), ("Geannuleerd: Overleg ontwerp", 1));
        assert_eq!((d.attachments[0].name.as_str(), d.attachments[0].mime.as_str()), (CANCEL_FILE, CANCEL_MIME));
        let text = String::from_utf8(d.attachments[0].bytes.to_vec()).unwrap().replace("\r\n ", "");
        assert!(text.contains("METHOD:CANCEL\r\n") && text.contains("STATUS:CANCELLED\r\n") && text.contains("mailto:ravi@example.org"), "{text}");
        assert!(!text.contains("sanne@"), "{text}");
        assert!(invitation::parse(&text).is_some_and(|i| i.cancelled));
        assert!(cancellation(&a, &[]).is_none());
    }

    #[test]
    fn the_answers_are_counted_in_words() {
        let mut a = meeting();
        let m = a.meeting.as_mut().unwrap();
        assert_eq!(tally_line(m), "2 nog geen reactie");
        m.attendees[0].response = Response::Accepted;
        assert_eq!(tally_line(m), "1 geaccepteerd, 1 nog geen reactie");
        assert_eq!((response_mark(Response::Declined), response_label(Response::Tentative)), ("✕", "Voorlopig"));
    }

    #[test]
    fn forwarding_publishes_the_appointment_without_organiser_or_attendees() {
        let text = published(&meeting()).replace("\r\n ", "");
        assert!(text.contains("METHOD:PUBLISH\r\n") && text.contains("SUMMARY:Overleg ontwerp\r\n"), "{text}");
        assert!(!text.contains("ORGANIZER") && !text.contains("ATTENDEE") && !text.contains("REQUEST"), "{text}");
        let inv = invitation::parse(&text).expect("our own reading understands it");
        assert_eq!((inv.title.as_str(), inv.cancelled), ("Overleg ontwerp", false));
        assert_eq!(inv.appointment.map(|a| (a.start, a.end)), Some((1_792_000_000, 1_792_003_600)));

        let d = forward(&appointment());
        assert_eq!((d.subject.as_str(), d.to.len(), d.attachments.len()), ("Doorgestuurd: Overleg ontwerp", 0, 1));
        assert_eq!((d.attachments[0].name.as_str(), d.attachments[0].mime.as_str()), (FORWARD_FILE, FORWARD_MIME));
        assert!(
            d.body.starts_with("Afspraak: Overleg ontwerp\nWanneer: ") && d.body.contains("Waar: Kamer 2\n") && d.body.contains("Schetsen meenemen."),
            "{}",
            d.body
        );
    }

    #[test]
    fn the_attendee_line_parses() {
        assert!(attendees("  ").unwrap().is_empty());
        assert_eq!(attendees("Sanne <sanne@example.org>; ravi@example.org").unwrap().len(), 2);
        assert_eq!(attendees("sanne@"), Err(BAD_ADDRESSES.to_string()));
    }

    #[test]
    fn only_a_real_account_can_send_an_invitation() {
        let session = mailcraft_engine::Engine::sample().session;
        assert!(account_for(&session, Some("local")).is_none(), "the sample account sends nothing");
        assert!(account_for(&session, None).is_none());
    }
}
