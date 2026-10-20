//! A meeting invitation in the mail, as Outlook shows one: a `text/calendar` part (an `.ics`)
//! with a VEVENT becomes a strip over the message that names the appointment, when and where
//! it is and who asks, with "Aan agenda toevoegen", which opens the appointment window filled in
//! so the person looks before it is saved. Nothing is answered to the organiser: Mailcraft sends
//! no acceptance, it only files the appointment in the person's own calendar. An answer to a
//! meeting the person organises shows as Outlook shows it, "Sanne heeft geaccepteerd", and the
//! engine has already filed it in the appointment.

use chrono::NaiveDateTime;
use egui::RichText;
use mailcraft_dav::ical;
use mailcraft_engine::meetings::{self, Answer};
use mailcraft_mime::Attachment;
use mailcraft_model::{Appointment, Response};

use crate::{MailApp, Tokens, calendar, dates};

/// Largest calendar part read, in bytes.
const MAX_ICS: usize = 512 * 1024;

/// What the invitation says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    /// The appointment as the invitation describes it; `None` when it was cancelled or has no
    /// usable start.
    pub appointment: Option<Appointment>,
    pub title: String,
    /// The organiser's name, or address, when the invitation names one.
    pub organizer: String,
    /// The invitation withdraws the appointment (METHOD:CANCEL or STATUS:CANCELLED).
    pub cancelled: bool,
    /// An attendee's answer (METHOD:REPLY or COUNTER) instead of an invitation.
    pub answer: Option<Answer>,
}

/// Whether the attachment is a calendar part.
pub fn is_calendar(a: &Attachment) -> bool {
    (a.mime.eq_ignore_ascii_case("text/calendar") || a.mime.eq_ignore_ascii_case("application/ics") || a.name.to_ascii_lowercase().ends_with(".ics"))
        && !a.bytes.is_empty()
        && a.bytes.len() <= MAX_ICS
}

/// The first invitation among the attachments, if any.
pub fn from_attachments(attachments: &[Attachment]) -> Option<Invitation> {
    attachments.iter().filter(|a| is_calendar(a)).find_map(|a| std::str::from_utf8(&a.bytes).ok().and_then(parse))
}

/// The invitation in an iCalendar text: the first VEVENT of the first VCALENDAR.
pub fn parse(text: &str) -> Option<Invitation> {
    let roots = ical::parse(text);
    let root = roots.iter().find(|c| c.name.eq_ignore_ascii_case("VCALENDAR"))?;
    let event = root.find("VEVENT")?;
    let title = event.text("SUMMARY");
    let cancelled = root.text("METHOD").eq_ignore_ascii_case("CANCEL") || event.text("STATUS").eq_ignore_ascii_case("CANCELLED");
    let organizer = event
        .prop("ORGANIZER")
        .map(|p| {
            let name = p.param("CN").map(|n| n.trim_matches('"').trim().to_string()).filter(|n| !n.is_empty());
            let addr = p.text();
            let addr = addr.strip_prefix("mailto:").or_else(|| addr.strip_prefix("MAILTO:")).unwrap_or(&addr).trim().to_string();
            name.unwrap_or(addr)
        })
        .unwrap_or_default();
    let appointment = if cancelled { None } else { ical::appointment_of(event, String::new()) };
    if title.trim().is_empty() && appointment.is_none() {
        return None;
    }
    let answer = meetings::parse_answer(text);
    Some(Invitation { title: if title.trim().is_empty() { "(zonder titel)".to_string() } else { title }, appointment, organizer, cancelled, answer })
}

/// "woensdag 21 oktober 10:00–11:30", or the days of an all-day appointment.
pub fn when(a: &Appointment) -> String {
    let start: NaiveDateTime = calendar::to_local(a.start);
    if a.all_day {
        let last = calendar::to_local(a.end - 1).date();
        if last == start.date() { dates::day_name(start.date()) } else { format!("{} – {}", dates::day_name(start.date()), dates::day_name(last)) }
    } else {
        let end = calendar::to_local(a.end);
        if end.date() == start.date() {
            format!("{} {}–{}", dates::day_name(start.date()), start.format("%H:%M"), end.format("%H:%M"))
        } else {
            format!("{} {} – {} {}", dates::day_name(start.date()), start.format("%H:%M"), dates::day_name(end.date()), end.format("%H:%M"))
        }
    }
}

/// "Sanne de Vries heeft geaccepteerd" and the like, for an answer.
pub fn answer_head(answer: &Answer) -> String {
    let who = answer.attendee.label();
    if answer.proposed.is_some() {
        return format!("{who} stelt een nieuwe tijd voor");
    }
    match answer.attendee.response {
        Response::Accepted => format!("{who} heeft geaccepteerd"),
        Response::Tentative => format!("{who} heeft voorlopig geaccepteerd"),
        Response::Declined => format!("{who} heeft afgewezen"),
        Response::None => format!("{who} heeft nog niet besloten"),
    }
}

/// The strip over an answer: who answered what, for which meeting and time; `filed` says the
/// meeting in a calendar holds this answer.
fn answer_strip(ui: &mut egui::Ui, t: &Tokens, inv: &Invitation, answer: &Answer, filed: bool) {
    let (mark, color) = match answer.attendee.response {
        Response::Accepted => ("✔", t.success),
        Response::Declined => ("✕", t.flag),
        _ => ("?", t.text_dim),
    };
    egui::Frame::new().fill(t.checked).corner_radius(4).inner_margin(8).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(mark).strong().color(color));
            ui.label(RichText::new(format!("{}: {}", answer_head(answer), inv.title)).strong().color(t.text));
        });
        let mut line = match (answer.proposed, &inv.appointment) {
            (Some((start, end)), Some(a)) => format!("Voorgesteld: {}", when(&Appointment { start, end, all_day: false, ..a.clone() })),
            (None, Some(a)) => when(a),
            _ => String::new(),
        };
        let filed = if filed { "Bijgewerkt in je agenda." } else { "Deze vergadering staat niet in je agenda." };
        line = if line.is_empty() { filed.to_string() } else { format!("{line} · {filed}") };
        ui.label(RichText::new(line).size(12.0).color(t.text_dim));
    });
}

/// The strip over the message; `in_calendar` says an appointment with this title and start is
/// already in a calendar (for an answer: that the meeting holds it). Returns the appointment to
/// add when the button was clicked.
pub fn strip(ui: &mut egui::Ui, t: &Tokens, inv: &Invitation, in_calendar: bool) -> Option<Appointment> {
    if let Some(answer) = &inv.answer {
        answer_strip(ui, t, inv, answer, in_calendar);
        return None;
    }
    let mut add = None;
    let fill = if inv.cancelled { t.flag.gamma_multiply(0.15) } else { t.checked };
    egui::Frame::new().fill(fill).corner_radius(4).inner_margin(8).show(ui, |ui| {
        let head = if inv.cancelled { format!("📅 Afspraak geannuleerd: {}", inv.title) } else { format!("📅 Uitnodiging: {}", inv.title) };
        ui.label(RichText::new(head).strong().color(if inv.cancelled { t.flag } else { t.text }));
        if let Some(a) = &inv.appointment {
            let mut line = when(a);
            if !a.location.trim().is_empty() {
                line.push_str(&format!(" · {}", a.location.trim()));
            }
            if !inv.organizer.is_empty() {
                line.push_str(&format!(" · van {}", inv.organizer));
            }
            ui.label(RichText::new(line).size(12.0).color(t.text_dim));
            ui.horizontal_wrapped(|ui| {
                if in_calendar {
                    ui.label(RichText::new("Staat al in je agenda.").size(12.0).color(t.text_dim));
                } else if ui
                    .small_button("Aan agenda toevoegen")
                    .on_hover_text("Opent het afspraakvenster, ingevuld; de organisator krijgt geen antwoord.")
                    .clicked()
                {
                    add = Some(a.clone());
                }
            });
        } else if !inv.organizer.is_empty() {
            ui.label(RichText::new(format!("van {}", inv.organizer)).size(12.0).color(t.text_dim));
        }
    });
    add
}

impl MailApp {
    /// Whether a calendar already holds an appointment with this title at this start.
    pub fn has_appointment_like(&self, a: &Appointment) -> bool {
        self.engine.session.appointments.iter().any(|b| b.start == a.start && b.title.trim().eq_ignore_ascii_case(a.title.trim()))
    }

    /// Whether a meeting in a calendar holds `answer`: the attendee's answer, and a proposed time.
    pub fn has_answer(&self, answer: &Answer) -> bool {
        self.engine.session.appointments.iter().filter_map(|a| a.meeting.as_ref()).filter(|m| m.uid == answer.uid).any(|m| {
            m.attendees.iter().any(|p| {
                p.email.eq_ignore_ascii_case(&answer.attendee.email)
                    && (p.response == answer.attendee.response || answer.proposed.is_some())
                    && (answer.proposed.is_none() || p.proposed == answer.proposed)
            })
        })
    }

    /// Opens the appointment window with the invitation filled in, for the person's own calendar.
    pub fn add_invitation(&mut self, a: &Appointment) {
        let mut e = calendar::Editor::from_appointment(a, None);
        e.id = None;
        e.calendar_id = None;
        self.calendar.editor = Some(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:k1@x\r\nDTSTAMP:20261001T120000Z\r\nDTSTART:20261021T080000Z\r\nDTEND:20261021T093000Z\r\nSUMMARY:Kickoff\r\nLOCATION:Kantoor\r\nORGANIZER;CN=\"Sanne de Vries\":mailto:sanne@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn ics(text: &str) -> Attachment {
        Attachment { name: "invite.ics".into(), mime: "text/calendar".into(), size: text.len(), bytes: std::sync::Arc::from(text.as_bytes()) }
    }

    #[test]
    fn an_invitation_names_the_appointment_the_organizer_and_a_cancellation() {
        let inv = from_attachments(&[ics(REQUEST)]).expect("a request");
        let a = inv.appointment.as_ref().expect("an appointment");
        assert_eq!((inv.title.as_str(), inv.organizer.as_str(), inv.cancelled), ("Kickoff", "Sanne de Vries", false));
        assert_eq!((a.location.as_str(), a.end - a.start, a.all_day), ("Kantoor", 5400, false));
        assert!(when(a).contains("21 oktober") && when(a).contains('–'));

        let cancel = REQUEST.replace("METHOD:REQUEST", "METHOD:CANCEL");
        let inv = parse(&cancel).expect("a cancellation");
        assert!(inv.cancelled && inv.appointment.is_none() && inv.title == "Kickoff");

        assert!(parse("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n").is_none(), "no event, no strip");
        assert!(
            from_attachments(&[Attachment { name: "bon.pdf".into(), mime: "application/pdf".into(), ..ics(REQUEST) }]).is_none(),
            "a PDF is not read as a calendar"
        );
        let big = Attachment { size: MAX_ICS + 1, bytes: std::sync::Arc::from(vec![b'x'; MAX_ICS + 1]), ..ics(REQUEST) };
        assert!(!is_calendar(&big));
    }
}
