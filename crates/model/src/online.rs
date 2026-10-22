//! Online meetings: the link to join one. Microsoft 365 and Google name it with the event;
//! otherwise it sits in an invitation's location or notes, where Merel finds it by the host of
//! the services people meet with.

use serde::{Deserialize, Serialize};

/// The longest meeting link kept, in characters.
pub const MAX_LINK: usize = 2048;

/// The service a meeting link belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    Microsoft365,
    Zoom,
    GoogleMeet,
    Jitsi,
    Webex,
    Whereby,
    /// A link the server named, from a host Merel does not know.
    Other,
}

impl Service {
    /// The service of a host, when it is one people meet with.
    fn of_host(host: &str) -> Option<Service> {
        let is = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
        Some(if is("teams.microsoft.com") || is("teams.live.com") || is("teams.microsoft.us") {
            Service::Microsoft365
        } else if is("zoom.us") || is("zoomgov.com") {
            Service::Zoom
        } else if host == "meet.google.com" {
            Service::GoogleMeet
        } else if host == "meet.jit.si" || host == "8x8.vc" {
            Service::Jitsi
        } else if is("webex.com") {
            Service::Webex
        } else if is("whereby.com") {
            Service::Whereby
        } else {
            return None;
        })
    }

    /// The service's name as people know it; "Microsoft 365" for the one Outlook uses.
    pub fn name(self) -> &'static str {
        match self {
            Service::Microsoft365 => "Microsoft 365",
            Service::Zoom => "Zoom",
            Service::GoogleMeet => "Google Meet",
            Service::Jitsi => "Jitsi Meet",
            Service::Webex => "Webex",
            Service::Whereby => "Whereby",
            Service::Other => "",
        }
    }
}

/// The way into an online meeting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingLink {
    pub url: String,
    pub service: Service,
}

/// `url` as a meeting link: an `https` address of at most [`MAX_LINK`] characters, without
/// spaces, control characters or a user name before the host. A host Merel does not know
/// counts too, as [`Service::Other`], since this is for a link the server or the person names.
pub fn meeting_link(url: &str) -> Option<MeetingLink> {
    let url = url.trim();
    let rest = url.get(..8).filter(|s| s.eq_ignore_ascii_case("https://")).map(|_| &url[8..])?;
    if url.chars().count() > MAX_LINK || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return None;
    }
    let host = crate::host_of(url)?;
    Some(MeetingLink { url: url.to_string(), service: Service::of_host(&host).unwrap_or(Service::Other) })
}

/// The first link in `text` to a service people meet with (see [`Service`]); a link to any
/// other site is not taken for one.
pub fn find_meeting_link(text: &str) -> Option<MeetingLink> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("https://").map(|i| from + i) {
        let end = text[at..].find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '|')).map_or(text.len(), |i| at + i);
        let candidate = text[at..end].trim_end_matches(['.', ',', ';', ':', ')', ']', '!', '?']);
        if let Some(link) = meeting_link(candidate).filter(|l| l.service != Service::Other) {
            return Some(link);
        }
        from = end.max(at + 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_services_are_found_in_text_and_others_are_not() {
        let notes = "Doe mee via <https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%7d>\nVergadering-ID: 342";
        let link = find_meeting_link(notes).unwrap();
        assert_eq!(link.service, Service::Microsoft365);
        assert!(link.url.ends_with("%7d"), "{}", link.url);

        let zoom = find_meeting_link("Zie https://example.org/agenda, en daarna https://us02web.zoom.us/j/8812345?pwd=xyz.").unwrap();
        assert_eq!((zoom.service, zoom.url.as_str()), (Service::Zoom, "https://us02web.zoom.us/j/8812345?pwd=xyz"));
        assert_eq!(find_meeting_link("Meet: HTTPS://meet.google.com/abc-defg-hij").map(|l| l.service), Some(Service::GoogleMeet));
        assert_eq!(find_meeting_link("Alleen https://example.org/zoom.us en http://meet.jit.si/kamer"), None);
        // A host that only ends in a service's name is someone else's.
        assert_eq!(find_meeting_link("https://notzoom.us/j/1"), None);
    }

    #[test]
    fn a_named_link_must_be_a_plain_https_address() {
        assert_eq!(meeting_link(" https://meet.example.org/kamer ").map(|l| l.service), Some(Service::Other));
        assert_eq!(meeting_link("http://teams.microsoft.com/l/x"), None);
        assert_eq!(meeting_link("https://teams.microsoft.com@evil.example/x"), None);
        assert_eq!(meeting_link("https://zoom.us/j/1 extra"), None);
        assert_eq!(meeting_link("javascript:alert(1)"), None);
        assert_eq!(meeting_link(&format!("https://zoom.us/j/{}", "1".repeat(MAX_LINK))), None);
    }
}
