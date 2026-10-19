//! Signs of phishing in a message, as warnings for the reading pane: a sender who poses as a
//! contact, a domain one letter away from one you know, a server that reports failed
//! authentication, a link whose text shows another site than it goes to, an attachment that is
//! a program. Nothing here decides for the person or sends anything anywhere; it points.

use crate::{Address, Contact};

/// File extensions of attachments that run as a program when opened.
pub const RISKY_EXTENSIONS: &[&str] = &[
    "exe", "scr", "bat", "cmd", "com", "pif", "msi", "msp", "js", "jse", "vbs", "vbe", "wsf", "wsh", "ps1", "psm1", "jar", "hta", "lnk", "dll",
    "reg", "iso", "img", "vhd", "vhdx", "apk", "app", "dmg", "pkg", "cpl", "inf", "sct", "chm", "url", "html", "htm", "svg",
];

/// Domains shorter than this are not compared letter by letter: "a.nl" vs "b.nl" says nothing.
const MIN_LOOKALIKE_LEN: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Warning {
    /// The sender's name is a contact's, or looks like an address, while the real address is
    /// another.
    Impersonation { name: String },
    /// The sender's domain is one letter (or a look-alike letter) away from a domain you know.
    LookalikeDomain { domain: String, like: String },
    /// The receiving server reported failed authentication checks.
    Unauthenticated { checks: String },
    /// A link whose text shows one site while it goes to another host.
    MisleadingLink { shown: String, host: String },
    /// An attachment that runs as a program when opened.
    RiskyAttachment { name: String },
}

impl Warning {
    /// One Dutch sentence for the reading pane.
    pub fn text(&self) -> String {
        match self {
            Warning::Impersonation { name } => format!("De afzender noemt zich \"{name}\", maar het adres is een ander dan je van die naam kent."),
            Warning::LookalikeDomain { domain, like } => format!("Het domein {domain} lijkt op {like}, maar is het niet."),
            Warning::Unauthenticated { checks } => {
                format!("Je mailserver kon niet bevestigen dat dit bericht echt van de afzender komt ({checks} mislukt).")
            }
            Warning::MisleadingLink { shown, host } => format!("Een link toont {shown} maar gaat naar {host}."),
            Warning::RiskyAttachment { name } => format!("De bijlage {name} is een programma; open die niet."),
        }
    }
}

/// The lower-case domain of an address.
fn domain_of(email: &str) -> String {
    email.rsplit_once('@').map(|(_, d)| d).unwrap_or("").trim_end_matches('.').to_ascii_lowercase()
}

/// Letters that are swapped for look-alikes in a forged domain, folded to one spelling.
fn fold_lookalikes(domain: &str) -> String {
    let mut s = domain.replace("rn", "m").replace("vv", "w").replace("cl", "d");
    s = s
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | 'i' | '!' | '|' => 'l',
            '3' => 'e',
            '5' => 's',
            '7' => 't',
            '-' | '_' => '\0',
            c => c,
        })
        .filter(|c| *c != '\0')
        .collect();
    s
}

/// The number of single-character edits between `a` and `b`, where swapping two neighbours
/// counts as one (the typo-squatter's favourite).
fn edits(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut rows: Vec<Vec<usize>> = vec![(0..=b.len()).collect()];
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let mut d = (rows[i][j] + cost).min(rows[i][j + 1] + 1).min(cur[j] + 1);
            if i > 0 && j > 0 && *ca == b[j - 1] && a[i - 1] == *cb {
                d = d.min(rows[i - 1][j - 1] + 1);
            }
            cur.push(d);
        }
        rows.push(cur);
    }
    rows[a.len()][b.len()]
}

/// Whether `domain` could be mistaken for `known` without being it or one of its subdomains.
pub fn lookalike(domain: &str, known: &str) -> bool {
    if domain == known || domain.ends_with(&format!(".{known}")) || known.ends_with(&format!(".{domain}")) {
        return false;
    }
    if fold_lookalikes(domain) == fold_lookalikes(known) {
        return true;
    }
    domain.len() >= MIN_LOOKALIKE_LEN && known.len() >= MIN_LOOKALIKE_LEN && edits(domain, known) == 1
}

/// What the sender's name and address say about the sender, against the contacts and the
/// person's own addresses.
pub fn sender_warnings(from: &Address, contacts: &[Contact], own_addresses: &[&str]) -> Vec<Warning> {
    let mut out = Vec::new();
    let email = from.email().to_ascii_lowercase();
    if let Some(name) = from.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        let lower = name.to_ascii_lowercase();
        let looks_like_address = lower.contains('@') && !lower.contains(char::is_whitespace);
        let posing = if looks_like_address {
            lower.trim_matches(['<', '>']) != email
        } else {
            contacts.iter().any(|c| c.name.trim().eq_ignore_ascii_case(name) && !c.emails.iter().any(|e| e.trim().eq_ignore_ascii_case(&email)))
        };
        if posing {
            out.push(Warning::Impersonation { name: name.to_string() });
        }
    }
    let domain = domain_of(&email);
    if !domain.is_empty() {
        let mut known: Vec<String> =
            contacts.iter().flat_map(|c| c.emails.iter().map(String::as_str)).chain(own_addresses.iter().copied()).map(domain_of).collect();
        known.sort();
        known.dedup();
        if !known.contains(&domain)
            && let Some(like) = known.iter().find(|k| !k.is_empty() && lookalike(&domain, k))
        {
            out.push(Warning::LookalikeDomain { domain, like: like.clone() });
        }
    }
    out
}

/// What the receiving server's `Authentication-Results` headers say: a warning when DMARC
/// failed, or when SPF or DKIM failed and DMARC did not pass. No headers, no opinion.
pub fn authentication_warning(results: &[String]) -> Option<Warning> {
    let all = results.join(" ").to_ascii_lowercase();
    let has = |needle: &str| all.split([' ', ';', ',', '(', ')', '\t', '\n']).any(|t| t == needle);
    if has("dmarc=fail") {
        return Some(Warning::Unauthenticated { checks: "DMARC".into() });
    }
    if has("dmarc=pass") {
        return None;
    }
    let mut failed = Vec::new();
    if has("spf=fail") || has("spf=softfail") {
        failed.push("SPF");
    }
    if has("dkim=fail") {
        failed.push("DKIM");
    }
    (!failed.is_empty()).then(|| Warning::Unauthenticated { checks: failed.join(" en ") })
}

/// The lower-case host of a URL (or of bare text such as "www.bank.nl/login"), without
/// credentials and port.
pub fn host_of(url: &str) -> Option<String> {
    let s = url.trim();
    let rest = s.split_once("://").map(|(scheme, rest)| if scheme.contains(['/', '.', ' ']) { s } else { rest }).unwrap_or(s);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = host.rsplit_once(':').filter(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())).map(|(h, _)| h).unwrap_or(host);
    let host = host.trim_matches('.').to_ascii_lowercase();
    (!host.is_empty() && host.contains('.') && !host.contains(char::is_whitespace)).then_some(host)
}

/// Whether `text`, as a link shows it, reads as a web address.
fn reads_as_url(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    !t.contains(char::is_whitespace)
        && (t.starts_with("http://")
            || t.starts_with("https://")
            || t.starts_with("www.")
            || host_of(&t).is_some_and(|h| h.len() == t.trim_end_matches('/').len()))
}

/// The part of a host that is registered: the last two labels, or three when the second to
/// last is a short public one ("co.uk", "com.au").
fn site_of(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    let keep = if labels.len() > 2 && labels[labels.len() - 1].len() == 2 && labels[labels.len() - 2].len() <= 3 { 3 } else { 2 };
    labels[labels.len().saturating_sub(keep)..].join(".")
}

/// Whether `a` and `b` belong to the same site, "www.bank.nl" and "login.bank.nl" included.
fn same_site(a: &str, b: &str) -> bool {
    a == b || site_of(a) == site_of(b)
}

/// Links (text, href) whose text names another site than the href goes to.
pub fn link_warnings(links: &[(String, String)]) -> Vec<Warning> {
    let mut out: Vec<Warning> = Vec::new();
    for (text, href) in links {
        let lower = href.trim().to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) || !reads_as_url(text) {
            continue;
        }
        let (Some(host), Some(shown)) = (host_of(href), host_of(text)) else { continue };
        if !same_site(&shown, &host) {
            let w = Warning::MisleadingLink { shown, host };
            if !out.contains(&w) {
                out.push(w);
            }
        }
    }
    out
}

/// Attachments that run as a program when opened, by their last extension.
pub fn attachment_warnings<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<Warning> {
    names
        .into_iter()
        .filter(|n| {
            let ext = n.trim().rsplit_once('.').map(|(_, e)| e.trim().to_ascii_lowercase()).unwrap_or_default();
            RISKY_EXTENSIONS.contains(&ext.as_str())
        })
        .map(|n| Warning::RiskyAttachment { name: n.to_string() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(name: Option<&str>, email: &str) -> Address {
        Address::new(name, email).unwrap()
    }

    fn contact(name: &str, email: &str) -> Contact {
        Contact { id: "c".into(), name: name.into(), emails: vec![email.into()], ..Contact::default() }
    }

    #[test]
    fn a_sender_posing_as_a_contact_or_as_an_address_is_pointed_out() {
        let contacts = [contact("Sanne de Vries", "sanne@example.org")];
        assert!(sender_warnings(&addr(Some("Sanne de Vries"), "sanne@example.org"), &contacts, &[]).is_empty());
        assert!(sender_warnings(&addr(Some("Sanne de Vries"), "SANNE@Example.org"), &contacts, &[]).is_empty(), "case does not matter");
        assert_eq!(
            sender_warnings(&addr(Some("sanne de vries"), "sanne.devries@mail.example"), &contacts, &[]),
            [Warning::Impersonation { name: "sanne de vries".into() }]
        );
        assert_eq!(
            sender_warnings(&addr(Some("bank@ing.nl"), "x@attacker.example"), &[], &[]),
            [Warning::Impersonation { name: "bank@ing.nl".into() }],
            "a name that is another address"
        );
        assert!(sender_warnings(&addr(Some("x@attacker.example"), "x@attacker.example"), &[], &[]).is_empty());
        assert!(sender_warnings(&addr(None, "nobody@nowhere.example"), &contacts, &[]).is_empty());
    }

    #[test]
    fn a_lookalike_domain_is_pointed_out_but_subdomains_and_strangers_are_not() {
        assert!(lookalike("rab0bank.nl", "rabobank.nl"), "a zero for an o");
        assert!(lookalike("examp1e.org", "example.org"));
        assert!(lookalike("exarnple.org", "example.org"), "rn for m");
        assert!(lookalike("exampel.org", "example.org"), "one edit");
        assert!(!lookalike("mail.example.org", "example.org"), "a subdomain is the same site");
        assert!(!lookalike("example.org", "example.org"));
        assert!(!lookalike("other.org", "example.org"));
        assert!(!lookalike("a.nl", "b.nl"), "too short to say anything");
        let contacts = [contact("Sanne", "sanne@example.org")];
        assert_eq!(
            sender_warnings(&addr(Some("Facturen"), "info@examp1e.org"), &contacts, &["ik@mijnbedrijf.nl"]),
            [Warning::LookalikeDomain { domain: "examp1e.org".into(), like: "example.org".into() }]
        );
        assert_eq!(
            sender_warnings(&addr(None, "hr@mijnbedrijf.nl.evil.example"), &[], &["ik@mijnbedrijf.nl"]),
            [],
            "a longer domain that merely contains yours is not one edit away"
        );
        assert!(sender_warnings(&addr(None, "sanne@example.org"), &contacts, &[]).is_empty(), "the known domain itself");
    }

    #[test]
    fn failed_server_checks_give_one_warning() {
        let r = |s: &str| vec![s.to_string()];
        assert_eq!(authentication_warning(&[]), None, "no header, no opinion");
        assert_eq!(authentication_warning(&r("mx.example; spf=pass smtp.mailfrom=a.nl; dkim=pass; dmarc=pass")), None);
        assert_eq!(
            authentication_warning(&r("mx.example; spf=fail smtp.mailfrom=a.nl; dkim=pass; dmarc=pass header.from=a.nl")),
            None,
            "DMARC passing settles it"
        );
        assert_eq!(
            authentication_warning(&r("mx.example;\n dmarc=fail (p=REJECT) header.from=a.nl")),
            Some(Warning::Unauthenticated { checks: "DMARC".into() })
        );
        assert_eq!(
            authentication_warning(&[String::from("mx.example; spf=softfail"), String::from("mx.example; dkim=fail")]),
            Some(Warning::Unauthenticated { checks: "SPF en DKIM".into() })
        );
        assert_eq!(authentication_warning(&r("mx.example; spf=pass; dkim=none")), None);
    }

    #[test]
    fn a_link_that_shows_one_site_and_goes_to_another_is_pointed_out() {
        assert_eq!(host_of("https://User:pw@www.Bank.nl:443/login?x=1"), Some("www.bank.nl".into()));
        assert_eq!(host_of("www.bank.nl/login"), Some("www.bank.nl".into()));
        assert_eq!(host_of("Klik hier"), None);
        assert_eq!(host_of("http://good.example@evil.example/"), Some("evil.example".into()), "what is before the @ is not the host");
        assert!(same_site("www.bank.nl", "login.bank.nl") && same_site("shop.example.co.uk", "example.co.uk") && !same_site("bank.nl", "bank.de"));
        let l = |t: &str, h: &str| (t.to_string(), h.to_string());
        let links = [
            l("Klik hier", "https://evil.example/x"),
            l("https://www.bank.nl/inloggen", "https://evil.example/x"),
            l("www.bank.nl", "https://login.bank.nl/"),
            l("bank.nl", "https://bank.nl/"),
            l("mail Sanne", "mailto:sanne@example.org"),
            l("https://www.bank.nl", "https://evil.example/y"),
        ];
        assert_eq!(
            link_warnings(&links),
            [Warning::MisleadingLink { shown: "www.bank.nl".into(), host: "evil.example".into() }],
            "once, for the same pair"
        );
    }

    #[test]
    fn program_attachments_are_pointed_out_by_their_last_extension() {
        let w = attachment_warnings(["factuur.pdf", "Factuur.PDF.exe", "foto.jpg", "script.js", "archief.zip", "pagina.html"]);
        let names: Vec<String> = w
            .into_iter()
            .map(|w| match w {
                Warning::RiskyAttachment { name } => name,
                other => other.text(),
            })
            .collect();
        assert_eq!(names, ["Factuur.PDF.exe", "script.js", "pagina.html"]);
        assert!(Warning::RiskyAttachment { name: "a.exe".into() }.text().contains("a.exe"));
    }
}
