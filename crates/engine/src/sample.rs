//! Built-in sample mail so the app has something to show before an account is added.

use mailcraft_model::{
    Account, Address, Appointment, Calendar, Category, Contact, Flags, Folder, FolderKind, Frequency, MessageHeader, Priority, Protocol, Recurrence,
    Server, Task, Tls,
};

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
        auth: mailcraft_model::Auth::Password,
        incoming: Some(Server { host: "imap.example.org".into(), port: 993, tls: Tls::Implicit }),
        outgoing: Some(Server { host: "smtp.example.org".into(), port: 465, tls: Tls::Implicit }),
        dav: None,
        signature: String::new(),
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
                remote_id: String::new(),
                from: addr(name, email)?,
                to: vec![me.clone()],
                subject: subject.into(),
                date: base - (i as i64) * 5400,
                preview: preview.into(),
                flags: Flags { seen, ..Flags::default() },
                has_attachments: att,
                message_id: Some(format!("<sample-{i}@example.org>")),
                categories: if i == 1 { vec!["Groene categorie".into()] } else { Vec::new() },
                importance: if i == 1 { mailcraft_model::Importance::High } else { mailcraft_model::Importance::Normal },
            })
        })
        .collect();

    let mut session = Session { accounts: vec![account], folders, messages, categories: Category::defaults(), ..Session::default() };
    session.set_calendar_data(crate::calendar::CalendarFile {
        calendars: vec![Calendar::local()],
        appointments: appointments(),
        dismissed: Vec::new(),
        colors: Default::default(),
    });
    session.set_contacts(contacts());
    session.set_tasks(tasks());
    // The sample tasks show what a due day looks like; their reminders are closed so the demo
    // does not nag about them.
    session.dismissed_task_reminders =
        session.tasks.iter().filter_map(|t| t.due.map(|due| crate::tasks::TaskReminderKey { id: t.id.clone(), due })).collect();
    Some(session)
}

fn tasks() -> Vec<Task> {
    use chrono::{Days, Local, TimeZone};
    let today = Local::now().date_naive();
    let due = |days: u64| {
        let date = today.checked_add_days(Days::new(days))?;
        Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).earliest().map(|d| d.timestamp())
    };
    [
        ("Offerte drukwerk beantwoorden", Some(0), Priority::High, false),
        ("Schetsen voor donderdag afmaken", Some(2), Priority::Normal, false),
        ("Portfolio bijwerken", None, Priority::Low, false),
        ("Welkomstmail lezen", Some(0), Priority::Normal, true),
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(i, (title, in_days, priority, done))| {
        Task {
            id: format!("sample-{i}"),
            title: title.into(),
            notes: String::new(),
            due: in_days.and_then(due),
            priority,
            done,
            completed: done.then(|| due(0)).flatten(),
            account_id: None,
            repeat: None,
            notes_rich: None,
        }
        .validated()
        .ok()
    })
    .collect()
}

fn contacts() -> Vec<Contact> {
    [
        ("Sanne de Vries", "sanne@example.org", "06 12345678", "Studio ArtCraft"),
        ("Ravi Kumar", "ravi@example.org", "020 1234567", "Drukkerij Kumar"),
        ("Studio ArtCraft", "studio@example.org", "", ""),
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(i, (name, email, phone, company))| {
        Contact {
            id: format!("sample-{i}"),
            name: name.into(),
            emails: vec![email.into()],
            phones: if phone.is_empty() { Vec::new() } else { vec![phone.into()] },
            company: company.into(),
            notes: String::new(),
            account_id: None,
            notes_rich: None,
        }
        .validated()
        .ok()
    })
    .collect()
}

/// Three appointments around today and a weekly one, so the calendar has something to show.
fn appointments() -> Vec<Appointment> {
    use chrono::{Datelike, Days, Local, TimeZone};
    let today = Local::now().date_naive();
    let at = |days: u64, hour: u32| {
        let date = today.checked_add_days(Days::new(days))?;
        let time = date.and_hms_opt(hour, 0, 0)?;
        Local.from_local_datetime(&time).earliest().map(|d| d.timestamp())
    };
    let plan =
        [("Overleg ontwerp", "Studio", 0, 10, 11, false), ("Lunch met Sanne", "De Kantine", 1, 12, 13, false), ("Studiodag", "", 2, 0, 24, true)];
    let mut list: Vec<Appointment> = plan
        .into_iter()
        .enumerate()
        .filter_map(|(i, (title, location, day, from, to, all_day))| {
            Some(Appointment {
                id: format!("sample-{i}"),
                calendar_id: "local".into(),
                title: title.into(),
                start: at(day, from)?,
                end: if to == 24 { at(day + 1, 0)? } else { at(day, to)? },
                all_day,
                location: location.into(),
                notes: String::new(),
                notes_rich: None,
                recurrence: None,
                reminder: None,
                private: false,
                show_as: mailcraft_model::ShowAs::Busy,
                categories: if i == 0 { vec!["Blauwe categorie".into()] } else { Vec::new() },
                meeting: None,
            })
        })
        .collect();
    // Every Monday at nine, from the Monday of this week on.
    let monday = today.checked_sub_days(Days::new(u64::from(today.weekday().num_days_from_monday())));
    let week_start = monday.and_then(|d| d.and_hms_opt(9, 0, 0)).and_then(|t| Local.from_local_datetime(&t).earliest()).map(|d| d.timestamp());
    if let Some(start) = week_start {
        list.push(Appointment {
            id: "sample-weekly".into(),
            calendar_id: "local".into(),
            title: "Weekstart".into(),
            start,
            end: start + 1800,
            all_day: false,
            location: "Studio".into(),
            notes: String::new(),
            notes_rich: None,
            recurrence: Some(Recurrence::every(Frequency::Weekly)),
            reminder: None,
            private: false,
            show_as: mailcraft_model::ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
        });
    }
    list
}
