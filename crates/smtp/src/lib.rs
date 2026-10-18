//! Sending mail over SMTP with `lettre`. TLS is mandatory: implicit TLS or STARTTLS, both
//! through rustls with the platform's certificate verifier. There is no plain-text path.

use std::time::Duration;

use lettre::message::{Mailbox, header::ContentType};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls as SmtpTls, TlsParameters};
use lettre::{Message, SmtpTransport, Transport};
use mailcraft_model::{Address, Draft, Server, Tls};
use mailcraft_secrets::Secret;

/// How to log in.
#[derive(Clone, Debug)]
pub enum Credential {
    Password(Secret),
    /// An OAuth 2.0 access token, sent with SASL XOAUTH2.
    OAuth2(Secret),
}

#[derive(Debug, thiserror::Error)]
pub enum SmtpError {
    #[error("the message could not be built: {0}")]
    Build(String),
    #[error("could not connect to {host}: {reason}")]
    Connect { host: String, reason: String },
    #[error("the server refused the user name or password")]
    Login,
    #[error("the server refused the message: {0}")]
    Refused(String),
}

fn mailbox(a: &Address) -> Result<Mailbox, SmtpError> {
    let addr = a.email().parse::<lettre::Address>().map_err(|e| SmtpError::Build(e.to_string()))?;
    Ok(Mailbox::new(a.name.clone(), addr))
}

/// A Message-ID under the sender's own domain: lettre's default would put the computer's
/// host name on the wire, which says more about the sender than a mail header should.
fn message_id(from: &Address) -> String {
    use std::hash::{BuildHasher, Hasher};
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(now);
    let a = h.finish();
    let b = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let domain = from.email().rsplit('@').next().unwrap_or("localhost");
    format!("<{now:x}.{a:016x}{b:016x}@{domain}>")
}

/// The RFC 5322 message for `draft`, as the server will receive it.
pub fn build(from: &Address, draft: &Draft) -> Result<Message, SmtpError> {
    if draft.to.is_empty() {
        return Err(SmtpError::Build("Vul ten minste één ontvanger in.".into()));
    }
    let mut b = Message::builder()
        .from(mailbox(from)?)
        .subject(draft.subject.trim())
        .header(ContentType::TEXT_PLAIN)
        .date_now()
        .message_id(Some(message_id(from)));
    for a in &draft.to {
        b = b.to(mailbox(a)?);
    }
    for a in &draft.cc {
        b = b.cc(mailbox(a)?);
    }
    if let Some(id) = &draft.in_reply_to {
        b = b.in_reply_to(id.clone()).references(id.clone());
    }
    b.body(draft.body.clone()).map_err(|e| SmtpError::Build(e.to_string()))
}

/// Sends `message` through `server` as `username`.
pub fn send(server: &Server, username: &str, credential: &Credential, message: &Message) -> Result<(), SmtpError> {
    let params = TlsParameters::new(server.host.clone()).map_err(|e| SmtpError::Connect { host: server.host.clone(), reason: e.to_string() })?;
    let tls = match server.tls {
        Tls::Implicit => SmtpTls::Wrapper(params),
        Tls::StartTls => SmtpTls::Required(params),
    };
    let (secret, mechanisms) = match credential {
        Credential::Password(p) => (p.expose().to_string(), vec![Mechanism::Plain, Mechanism::Login]),
        Credential::OAuth2(t) => (t.expose().to_string(), vec![Mechanism::Xoauth2]),
    };
    let transport = SmtpTransport::builder_dangerous(server.host.as_str())
        .port(server.port)
        .tls(tls)
        .credentials(Credentials::new(username.to_string(), secret))
        .authentication(mechanisms)
        .timeout(Some(Duration::from_secs(30)))
        .build();
    match transport.send(message) {
        Ok(_) => Ok(()),
        Err(e) if e.is_client() || e.is_permanent() && e.to_string().to_lowercase().contains("auth") => Err(SmtpError::Login),
        Err(e) if e.is_response() || e.is_permanent() || e.is_transient() => Err(SmtpError::Refused(e.to_string())),
        Err(e) => Err(SmtpError::Connect { host: server.host.clone(), reason: e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_plain_text_message_with_threading_headers() {
        let from = Address::new(Some("Jij"), "jij@example.org").unwrap();
        let draft = Draft {
            to: vec![Address::new(Some("Sanne"), "sanne@example.org").unwrap()],
            cc: vec![],
            subject: "Re: Planning".into(),
            body: "Prima, tien uur.\n".into(),
            in_reply_to: Some("<abc@example.org>".into()),
        };
        let m = build(&from, &draft).unwrap();
        let text = String::from_utf8(m.formatted()).unwrap();
        assert!(text.contains("From: \"Jij\" <jij@example.org>") || text.contains("From: Jij <jij@example.org>"));
        assert!(text.contains("To: \"Sanne\" <sanne@example.org>") || text.contains("To: Sanne <sanne@example.org>"));
        assert!(text.contains("In-Reply-To: <abc@example.org>"));
        assert!(text.contains("Message-ID: <") && text.contains("@example.org>"));
        assert_ne!(message_id(&from), message_id(&from));
        assert!(text.contains("Prima, tien uur."));
    }

    #[test]
    fn refuses_an_empty_recipient_list() {
        let from = Address::new(None, "jij@example.org").unwrap();
        assert!(build(&from, &Draft::default()).is_err());
    }
}
