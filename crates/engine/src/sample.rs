//! Built-in sample mail so the app has something to show before an account is added.

use mailcraft_model::{Account, Address, Flags, Folder, FolderKind, MessageHeader, Protocol, Server, Tls};

use crate::Session;

fn addr(name: &str, email: &str) -> Option<Address> {
    Address::new(Some(name), email).ok()
}

/// The sample session; empty if a sample address were ever rejected by the validator.
pub fn session() -> Session {
    build().unwrap_or_default()
}

fn build() -> Option<Session> {
    let me = addr("Jij", "jij@example.org")?;
    let account = Account {
        id: "sample".into(),
        name: "Voorbeeld".into(),
        address: me.clone(),
        username: "jij@example.org".into(),
        protocol: Protocol::ImapSmtp,
        incoming: Some(Server { host: "imap.example.org".into(), port: 993, tls: Tls::Implicit }),
        outgoing: Some(Server { host: "smtp.example.org".into(), port: 465, tls: Tls::Implicit }),
    };
    let folders = [
        ("inbox", "Postvak IN", FolderKind::Inbox),
        ("drafts", "Concepten", FolderKind::Drafts),
        ("sent", "Verzonden items", FolderKind::Sent),
        ("archive", "Archief", FolderKind::Archive),
        ("junk", "Ongewenste e-mail", FolderKind::Junk),
        ("deleted", "Verwijderde items", FolderKind::Deleted),
    ]
    .into_iter()
    .map(|(id, name, kind)| Folder {
        id: id.into(),
        account_id: account.id.clone(),
        name: name.into(),
        path: id.to_ascii_uppercase(),
        kind,
        unread: 0,
    })
    .collect();

    let base = 1_792_000_000; // October 2026
    let msgs = [
        (
            "Studio ArtCraft",
            "studio@example.org",
            "Welkom bij Mailcraft",
            "Fijn dat je er bent. Dit is voorbeeldmail; voeg een account toe om je eigen mail te zien.",
            false,
            false,
        ),
        ("Sanne de Vries", "sanne@example.org", "Planning voor donderdag", "Zullen we om tien uur beginnen? Ik neem de schetsen mee.", false, true),
        ("Ravi Kumar", "ravi@example.org", "Re: Offerte drukwerk", "Hierbij de aangepaste offerte, met de oplage die we bespraken.", true, false),
        (
            "Nieuwsbrief",
            "news@example.org",
            "Wat er deze maand speelt",
            "Nieuwe workshops, een tentoonstelling en tips voor je portfolio.",
            true,
            false,
        ),
    ];
    let messages = msgs
        .into_iter()
        .enumerate()
        .filter_map(|(i, (name, email, subject, preview, seen, att))| {
            Some(MessageHeader {
                id: format!("m{i}"),
                folder_id: "inbox".into(),
                uid: 0,
                from: addr(name, email)?,
                to: vec![me.clone()],
                subject: subject.into(),
                date: base - (i as i64) * 5400,
                preview: preview.into(),
                flags: Flags { seen, ..Flags::default() },
                has_attachments: att,
            })
        })
        .collect();

    Some(Session { accounts: vec![account], folders, messages, ..Session::default() })
}
