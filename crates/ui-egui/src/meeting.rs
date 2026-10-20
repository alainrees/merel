//! Meeting requests, as Outlook's "Uitnodigen": the people typed under Genodigden in the
//! appointment window get a message with the appointment as a `text/calendar` request, sent from
//! the account the calendar belongs to. Nothing goes out without "Verzenden" being pressed.

use mailcraft_dav::ical::{self, Prop};
use mailcraft_engine::{SAMPLE_ACCOUNT, Session};
use mailcraft_model::{Account, Address, Appointment, Draft, DraftAttachment, parse_address_list};

use crate::compose::{Compose, Kind};
use crate::invitation;

pub const FIELD: &str = "Genodigden";
pub const FILE: &str = "uitnodiging.ics";
pub const MIME: &str = "text/calendar; method=REQUEST; charset=utf-8";
pub const NO_ACCOUNT: &str = "Voeg eerst een echt account toe om een uitnodiging te kunnen versturen; de afspraak is nog niet opgeslagen.";
pub const BAD_ADDRESSES: &str = "Controleer de adressen bij Genodigden.";
/// The button in the window of a saved appointment that passes it on by mail.
pub const FORWARD: &str = "Doorsturen…";
pub const FORWARD_FILE: &str = "afspraak.ics";
pub const FORWARD_MIME: &str = "text/calendar; method=PUBLISH; charset=utf-8";
const NOTE: &str = "Genodigden: ";

/// The addresses typed under Genodigden, or what is wrong with them; empty for no invitation.
pub fn attendees(text: &str) -> Result<Vec<Address>, String> {
    parse_address_list(text).map_err(|_| BAD_ADDRESSES.to_string())
}

/// The account an invitation for `calendar_id` goes out from: the calendar's own account, else
/// the account of the open folder. `None` when there is only the sample.
pub fn account_for(session: &Session, calendar_id: Option<&str>) -> Option<Account> {
    let own = calendar_id.and_then(|c| session.appointment_account(c)).and_then(|id| session.accounts.iter().find(|a| a.id == id));
    own.or_else(|| session.current_account()).filter(|a| a.id != SAMPLE_ACCOUNT).cloned()
}

/// The notes with a line naming the attendees, so the appointment itself says who was invited.
pub fn noted(notes: &str, attendees: &[Address]) -> String {
    let line = format!("{NOTE}{}", attendees.iter().map(mailbox).collect::<Vec<_>>().join("; "));
    if notes.lines().any(|l| l.trim() == line) {
        return notes.to_string();
    }
    if notes.trim().is_empty() { line } else { format!("{}\n\n{line}", notes.trim_end()) }
}

/// The iCalendar request: the appointment as a VEVENT with the organiser and the attendees, and
/// `METHOD:REQUEST` so mail programs show it as an invitation.
pub fn ics(a: &Appointment, organizer: &Address, attendees: &[Address]) -> String {
    let id: String = a.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let domain = organizer.email().rsplit_once('@').map(|(_, d)| d).unwrap_or("local");
    let mut root = ical::new_calendar("VEVENT", &format!("{id}-{}@mailcraft.{domain}", a.start));
    root.props.push(Prop::new("METHOD", "REQUEST"));
    if let Some(event) = root.find_mut("VEVENT") {
        ical::write_appointment(event, a);
        event.set("STATUS", Some(Prop::new("STATUS", "CONFIRMED")));
        event.set("SEQUENCE", Some(Prop::new("SEQUENCE", "0")));
        event.set("ORGANIZER", Some(person("ORGANIZER", organizer)));
        for p in attendees {
            event.props.push(person("ATTENDEE", p).with("ROLE", "REQ-PARTICIPANT").with("PARTSTAT", "NEEDS-ACTION").with("RSVP", "TRUE"));
        }
    }
    root.serialize()
}

/// The message that carries the request: the appointment in words, for mail programs that do
/// not read the calendar part, and the request itself as the attachment.
pub fn draft(a: &Appointment, organizer: &Address, attendees: Vec<Address>) -> Draft {
    let mut body = format!("Je bent uitgenodigd voor: {}\nWanneer: {}\n", a.title.trim(), invitation::when(a));
    if !a.location.trim().is_empty() {
        body.push_str(&format!("Waar: {}\n", a.location.trim()));
    }
    body.push_str(&format!("Organisator: {}\n", mailbox(organizer)));
    if !a.notes.trim().is_empty() {
        body.push_str(&format!("\n{}\n", a.notes.trim()));
    }
    body.push_str("\nDe bijgevoegde uitnodiging zet de afspraak in je agenda.\n");
    let mut attachment = DraftAttachment::new(FILE, ics(a, organizer, &attendees).into_bytes());
    attachment.mime = MIME.to_string();
    Draft { to: attendees, subject: format!("Uitnodiging: {}", a.title.trim()), body, attachments: vec![attachment], ..Draft::default() }
}

/// The appointment as a published calendar file, for passing it on: no organiser, no attendees
/// and nothing to answer, so the reader's program simply offers to add it.
pub fn published(a: &Appointment) -> String {
    let id: String = a.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut root = ical::new_calendar("VEVENT", &format!("{id}-{}@mailcraft.local", a.start));
    root.props.push(Prop::new("METHOD", "PUBLISH"));
    if let Some(event) = root.find_mut("VEVENT") {
        ical::write_appointment(event, a);
    }
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

/// `ORGANIZER;CN=Naam:mailto:adres`, or an attendee the same way.
fn person(name: &str, a: &Address) -> Prop {
    let p = Prop::new(name, &format!("mailto:{}", a.email()));
    let cn = a.display().replace('"', "");
    if cn.is_empty() || cn == a.email() { p } else { p.with("CN", &cn) }
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
        }
    }

    #[test]
    fn the_request_names_the_organiser_the_attendees_and_the_method() {
        let me = Address::new(Some("Alain"), "alain@example.org").unwrap();
        let sanne = Address::new(Some("Sanne de Vries"), "sanne@example.org").unwrap();
        let ravi = Address::new(None, "ravi@example.org").unwrap();
        let text = ics(&appointment(), &me, &[sanne.clone(), ravi.clone()]).replace("\r\n ", "");
        assert!(text.contains("METHOD:REQUEST\r\n"), "{text}");
        assert!(text.contains("UID:a7-1792000000@mailcraft.example.org\r\n"), "{text}");
        assert!(text.contains("ORGANIZER;CN=Alain:mailto:alain@example.org\r\n"), "{text}");
        assert!(
            text.contains("ATTENDEE;CN=Sanne de Vries;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:sanne@example.org\r\n"),
            "{text}"
        );
        assert!(text.contains("ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:ravi@example.org\r\n"), "{text}");
        assert!(text.contains("SUMMARY:Overleg ontwerp\r\n") && text.contains("LOCATION:Kamer 2\r\n") && text.contains("STATUS:CONFIRMED\r\n"));
        // Our own reading of invitations understands it.
        let inv = invitation::parse(&text).expect("an invitation");
        assert_eq!((inv.title.as_str(), inv.organizer.as_str(), inv.cancelled), ("Overleg ontwerp", "Alain", false));
        assert_eq!(inv.appointment.map(|a| (a.start, a.end, a.location)), Some((1_792_000_000, 1_792_003_600, "Kamer 2".to_string())));

        let d = draft(&appointment(), &me, vec![sanne, ravi]);
        assert_eq!((d.subject.as_str(), d.to.len(), d.attachments.len()), ("Uitnodiging: Overleg ontwerp", 2, 1));
        assert_eq!((d.attachments[0].name.as_str(), d.attachments[0].mime.as_str()), (FILE, MIME));
        assert!(d.body.starts_with("Je bent uitgenodigd voor: Overleg ontwerp\nWanneer: ") && d.body.contains("Waar: Kamer 2\n"), "{}", d.body);
        assert!(d.body.contains("Organisator: Alain <alain@example.org>\n") && d.body.contains("\nSchetsen meenemen.\n"));
    }

    #[test]
    fn forwarding_publishes_the_appointment_without_organiser_or_attendees() {
        let text = published(&appointment()).replace("\r\n ", "");
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
    fn the_attendee_line_parses_and_is_noted_once() {
        assert!(attendees("  ").unwrap().is_empty());
        assert_eq!(attendees("Sanne <sanne@example.org>; ravi@example.org").unwrap().len(), 2);
        assert_eq!(attendees("sanne@"), Err(BAD_ADDRESSES.to_string()));
        let list = attendees("Sanne <sanne@example.org>, ravi@example.org").unwrap();
        let once = noted("Schetsen meenemen.", &list);
        assert_eq!(once, "Schetsen meenemen.\n\nGenodigden: Sanne <sanne@example.org>; ravi@example.org");
        assert_eq!(noted(&once, &list), once, "saving again does not repeat the line");
        assert_eq!(noted("", &list), "Genodigden: Sanne <sanne@example.org>; ravi@example.org");
    }

    #[test]
    fn only_a_real_account_can_send_an_invitation() {
        let session = mailcraft_engine::Engine::sample().session;
        assert!(account_for(&session, Some("local")).is_none(), "the sample account sends nothing");
        assert!(account_for(&session, None).is_none());
    }
}
