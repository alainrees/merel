//! One background thread per account, holding its connection and serving [`Job`]s in order.
//!
//! The thread speaks IMAP, Exchange Web Services, Microsoft Graph or Google's APIs through the
//! [`Mailbox`] trait and never knows which. It reconnects once when a command fails on a
//! dropped connection; after that the failure is reported and the next job tries again from
//! scratch. A change (flag, move, item) that finds no connection is handed back as
//! [`Event::Deferred`] for the offline queue, and the thread tries the server again on its
//! own, at growing intervals, until it is back.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Duration;

use merel_dav::DavMailbox;
use merel_exchange::EwsClient;
use merel_google::GoogleMailbox;
use merel_graph::GraphClient;
use merel_imap::ImapClient;
use merel_mailbox::{Mailbox, MailboxError};
use merel_model::{Account, Protocol, Server};
use merel_smtp::SmtpError;

use crate::certificates::{Keys, outgoing};
use crate::signin::Credentials;
use crate::{Event, Job};

/// First wait before trying the server again after a failed connection; it doubles each time.
const FIRST_RETRY: Duration = Duration::from_secs(15);
/// Longest wait between two tries.
const MAX_RETRY: Duration = Duration::from_secs(5 * 60);

pub struct Worker {
    events: Sender<Event>,
    accounts: HashMap<String, Sender<Job>>,
    keys: Keys,
}

impl Worker {
    /// `keys` are the S/MIME certificates (the accounts' own and the correspondents'), read
    /// when a message goes out.
    pub fn new(events: Sender<Event>, keys: Keys) -> Self {
        Worker { events, accounts: HashMap::new(), keys }
    }

    /// A sender for events from threads the engine starts itself.
    pub fn events(&self) -> Sender<Event> {
        self.events.clone()
    }

    /// Starts the thread for `account`. Jobs for it queue until the connection is up.
    pub fn start_account(&mut self, account: Account, credentials: Credentials) {
        let (tx, rx) = channel();
        let events = self.events.clone();
        let id = account.id.clone();
        let name = format!("mailbox-{id}");
        let keys = self.keys.clone();
        let run = move || {
            let id = account.id.clone();
            let reporter = events.clone();
            // A bug in a protocol library must not leave the account on "Verbinden…" for good.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| serve(account, credentials, keys, rx, events)));
            if let Err(panic) = outcome {
                let detail =
                    panic.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_default();
                log::error!("{id}: the connection thread stopped: {detail}");
                let what = format!("Merel liep vast bij het verbinden ({}). Herstart Merel; meld het als het blijft gebeuren.", clip(&detail));
                let _ = reporter.send(Event::Failed { account_id: id, what });
            }
        };
        if thread::Builder::new().name(name).spawn(run).is_ok() {
            self.accounts.insert(id, tx);
        } else {
            let _ = self.events.send(Event::Failed { account_id: id, what: "Kon geen achtergrondtaak starten.".into() });
        }
    }

    /// Ends the thread for `account_id`: dropping its queue makes the loop in `serve` return
    /// after the job in hand, and the connection is dropped with it.
    pub fn stop_account(&mut self, account_id: &str) {
        self.accounts.remove(account_id);
    }

    pub fn submit(&self, job: Job) {
        let id = job.account_id().to_string();
        if let Some(tx) = self.accounts.get(&id) {
            if tx.send(job).is_err() {
                let _ = self.events.send(Event::Failed { account_id: id, what: "De verbinding is gestopt.".into() });
            }
        } else {
            let _ = self.events.send(Event::Failed { account_id: id, what: "Dit account is niet verbonden.".into() });
        }
    }
}

fn describe(e: &MailboxError) -> String {
    match e {
        MailboxError::Connect { host, reason } => format!("Kan geen verbinding maken met {host}: {}.", clip(reason)),
        MailboxError::Login => "De server accepteert de gebruikersnaam of het wachtwoord niet.".into(),
        MailboxError::AuthScheme(scheme) => format!(
            "De server eist een aanmeldmethode die Merel nog niet ondersteunt ({scheme}). Vraag de beheerder om Basic-verificatie voor EWS, of gebruik een Microsoft 365-account."
        ),
        MailboxError::Protocol(m) => format!("De server gaf een fout: {}", clip(m)),
        MailboxError::BadName(_) => "Een naam is niet toegestaan.".into(),
    }
}

fn describe_smtp(e: &SmtpError) -> String {
    match e {
        SmtpError::Build(m) => m.clone(),
        SmtpError::Connect { host, .. } => format!("Kan geen verbinding maken met {host}."),
        SmtpError::Login => "De uitgaande server accepteert de gebruikersnaam of het wachtwoord niet.".into(),
        SmtpError::Refused(m) => format!("De server weigerde het bericht: {}", clip(m)),
    }
}

fn clip(s: &str) -> String {
    merel_model::clean_text(s).chars().take(160).collect()
}

/// A failure of `job`, reported where the person is looking: on the draft for a send, on the
/// account otherwise.
fn failure(account_id: &str, job: &Job, what: String) -> Event {
    match job {
        Job::Send { draft_id, .. } => Event::Sent { account_id: account_id.to_string(), draft_id: *draft_id, result: Err(what) },
        Job::Search { folder_id, query, .. } => Event::Found { folder_id: folder_id.clone(), query: query.clone(), result: Err(what) },
        _ => Event::Failed { account_id: account_id.to_string(), what },
    }
}

/// What a job that found no connection becomes: a change waits, anything else fails.
fn no_connection(account_id: &str, job: &Job, what: String) -> Event {
    if job.is_change() { Event::Deferred { account_id: account_id.to_string(), job: Box::new(job.clone()) } } else { failure(account_id, job, what) }
}

/// After a connection: says why the calendar and address book stayed away, when they did.
fn report_items_error(c: &dyn Mailbox, account_id: &str, events: &Sender<Event>) {
    if let Some(e) = c.items_error() {
        let what = format!("Agenda en adresboek: {}", describe(e));
        let _ = events.send(Event::ItemsFailed { account_id: account_id.to_string(), what });
    }
}

fn longer(wait: Option<Duration>) -> Option<Duration> {
    Some(wait.map_or(FIRST_RETRY, |w| (w * 2).min(MAX_RETRY)))
}

fn serve(account: Account, mut credentials: Credentials, keys: Keys, jobs: Receiver<Job>, events: Sender<Event>) {
    let Some(server) = account.incoming.clone() else {
        let _ = events.send(Event::Failed { account_id: account.id, what: "Dit account heeft geen inkomende server.".into() });
        return;
    };
    let mut client: Option<Box<dyn Mailbox>> = None;
    // Set after a failed connection: how long to wait for a job before trying the server again.
    let mut retry: Option<Duration> = None;
    loop {
        let job = match retry {
            None => match jobs.recv() {
                Ok(job) => job,
                Err(_) => break,
            },
            Some(wait) => match jobs.recv_timeout(wait) {
                Ok(job) => job,
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    // Nobody asked for anything; see whether the server is back. Connecting
                    // reports the folders, and the engine replays the offline queue.
                    match connect(&account, &server, &mut credentials) {
                        Ok(mut c) => match run(c.as_mut(), &account, &keys, &Job::Connect { account_id: account.id.clone() }) {
                            Ok(evs) => {
                                for ev in evs {
                                    let _ = events.send(ev);
                                }
                                report_items_error(c.as_ref(), &account.id, &events);
                                client = Some(c);
                                retry = None;
                            }
                            Err(e) => {
                                log::warn!("{}: {e} (reconnect)", account.id);
                                retry = longer(retry);
                            }
                        },
                        Err(_) => {
                            credentials.invalidate();
                            retry = longer(retry);
                        }
                    }
                    continue;
                }
            },
        };
        // Mail for an SMTP server goes out before the store is touched, so a dead IMAP
        // connection never holds up sending.
        if let Job::Send { draft_id, draft, sent_path, .. } = &job
            && account.outgoing.is_some()
        {
            let result = send_smtp(&account, &mut credentials, &keys, draft).map(|raw| {
                // A copy in Sent is nice to have; a failure there must not undo a successful send.
                if let (Some(path), Some(c)) = (sent_path, client.as_mut())
                    && let Err(e) = c.append(path, &raw)
                {
                    log::warn!("{}: could not file the sent message: {e}", account.id);
                }
            });
            let _ = events.send(Event::Sent { account_id: account.id.clone(), draft_id: *draft_id, result });
            continue;
        }
        // Connect lazily, and once more after a failure, so a dropped connection heals itself.
        let mut attempts = 0;
        loop {
            attempts += 1;
            if client.is_none() {
                match connect(&account, &server, &mut credentials) {
                    Ok(c) => client = Some(c),
                    Err(what) => {
                        // A refused login may be a stale access token: fetch a new one next time.
                        credentials.invalidate();
                        retry = longer(retry);
                        let _ = events.send(no_connection(&account.id, &job, what));
                        break;
                    }
                }
            }
            let Some(c) = client.as_mut() else { break };
            match run(c.as_mut(), &account, &keys, &job) {
                Ok(evs) => {
                    retry = None;
                    for ev in evs {
                        let _ = events.send(ev);
                    }
                    if matches!(job, Job::Connect { .. }) {
                        report_items_error(c.as_ref(), &account.id, &events);
                    }
                    break;
                }
                Err(e) => {
                    log::warn!("{}: {} (attempt {attempts})", account.id, e);
                    client = None;
                    if attempts >= 2 {
                        let ev = match e {
                            MailboxError::Connect { .. } => no_connection(&account.id, &job, describe(&e)),
                            _ => failure(&account.id, &job, describe(&e)),
                        };
                        let _ = events.send(ev);
                        break;
                    }
                }
            }
        }
    }
    if let Some(c) = client {
        c.logout();
    }
}

/// Builds, signs when asked, and sends `draft` over SMTP; returns the bytes that went out, for
/// the Sent folder.
fn send_smtp(account: &Account, credentials: &mut Credentials, keys: &Keys, draft: &merel_model::Draft) -> Result<Vec<u8>, String> {
    let server = account.outgoing.as_ref().ok_or_else(|| "Dit account heeft geen uitgaande server.".to_string())?;
    let message = merel_smtp::build(&account.address, draft).map_err(|e| describe_smtp(&e))?;
    let raw = outgoing(keys, account, draft, message.formatted())?;
    let credential = credentials.smtp()?;
    merel_smtp::send_raw(server, &account.username, &credential, &message, &raw).map_err(|e| describe_smtp(&e))?;
    Ok(raw)
}

fn connect(account: &Account, server: &Server, credentials: &mut Credentials) -> Result<Box<dyn Mailbox>, String> {
    let credential = credentials.incoming()?;
    Ok(match account.protocol {
        Protocol::Exchange => Box::new(EwsClient::connect(server, &account.username, &credential).map_err(|e| describe(&e))?),
        Protocol::Gmail => Box::new(GoogleMailbox::connect(server, &account.username, &credential).map_err(|e| describe(&e))?),
        // A Microsoft 365 account signed in for Graph; one from before that still speaks IMAP.
        Protocol::Microsoft365 if uses_graph(account) => Box::new(GraphClient::connect(server, &credential).map_err(|e| describe(&e))?),
        // An IMAP account with a CalDAV/CardDAV server named next to it.
        Protocol::ImapSmtp if account.dav.is_some() => {
            let dav = account.dav.as_ref().ok_or_else(|| "no DAV server".to_string())?;
            Box::new(DavMailbox::connect(server, &account.username, &credential, dav).map_err(|e| describe(&e))?)
        }
        Protocol::ImapSmtp | Protocol::Microsoft365 => {
            Box::new(ImapClient::connect(server, &account.username, &credential).map_err(MailboxError::from).map_err(|e| describe(&e))?)
        }
    })
}

/// The item store of `c`; an error for a store that keeps only mail, which the engine never
/// asks.
fn items_of(c: &mut dyn Mailbox) -> Result<&mut dyn merel_mailbox::ItemStore, MailboxError> {
    c.items().ok_or_else(|| MailboxError::Protocol("this account keeps no appointments, contacts or tasks".into()))
}

/// Whether the account's sign-in covers Microsoft Graph (an account signed in before Graph
/// came along has IMAP scopes, and keeps working as mail only until it signs in again).
fn uses_graph(account: &Account) -> bool {
    matches!(&account.auth, merel_model::Auth::OAuth2 { scope, .. } if scope.contains("graph.microsoft.com"))
}

/// Fetches an account's calendar, contacts and tasks. The calendar must come; contacts or tasks
/// the server refuses leave the rest standing, with a note on the account saying which part.
fn sync(store: &mut dyn merel_mailbox::ItemStore, account_id: &str, from: i64, to: i64) -> Result<Vec<Event>, MailboxError> {
    let appointments = store.appointments(from, to).map_err(|e| match e {
        MailboxError::Protocol(m) => MailboxError::Protocol(format!("{m} (agenda)")),
        e => e,
    })?;
    let mut refused = Vec::new();
    let contacts = part(store.contacts(), "Contactpersonen", &mut refused)?;
    let tasks = part(store.tasks(), "Taken", &mut refused)?;
    let mut events = vec![Event::Synced { account_id: account_id.to_string(), appointments, contacts, tasks }];
    if !refused.is_empty() {
        events.push(Event::ItemsFailed { account_id: account_id.to_string(), what: refused.join(" ") });
    }
    Ok(events)
}

/// One part of a sync: what the server refused becomes an empty list and a line in `refused`.
fn part<T>(result: Result<Vec<T>, MailboxError>, what: &str, refused: &mut Vec<String>) -> Result<Vec<T>, MailboxError> {
    match result {
        Err(e @ MailboxError::Protocol(_)) => {
            refused.push(format!("{what}: {}", describe(&e)));
            Ok(Vec::new())
        }
        other => other,
    }
}

fn run(c: &mut dyn Mailbox, account: &Account, keys: &Keys, job: &Job) -> Result<Vec<Event>, MailboxError> {
    if let Job::Sync { account_id, from, to } = job {
        return sync(items_of(c)?, account_id, *from, *to);
    }
    Ok(match job {
        Job::Connect { account_id } => {
            let folders = c.folders()?;
            Some(Event::Connected { account_id: account_id.clone(), folders, syncs_items: c.items().is_some() })
        }
        // Fetched in `sync` above.
        Job::Sync { .. } => None,
        Job::SaveItem { account_id, local_id, item } => {
            let remote_id = items_of(c)?.save(item)?;
            Some(Event::ItemSaved { account_id: account_id.clone(), local_id: local_id.clone(), remote_id })
        }
        Job::DeleteItem { id, .. } => {
            items_of(c)?.delete(id)?;
            None
        }
        Job::Headers { folder_id, path, offset, limit, .. } => {
            Some(Event::Headers { folder_id: folder_id.clone(), headers: c.headers(path, *offset, *limit)?, offset: *offset, limit: *limit })
        }
        Job::Search { folder_id, path, query, .. } => {
            let result = c.search(path, query, crate::SEARCH_LIMIT).map_err(|e| describe(&e));
            Some(Event::Found { folder_id: folder_id.clone(), query: query.clone(), result })
        }
        Job::Body { account_id, message_id, path, id } => {
            let raw = c.body(path, id)?;
            let (body, header, carried) = crate::certificates::read(keys, account_id, &raw);
            Some(Event::Body { message_id: message_id.clone(), body: Box::new(body), header: Box::new(header), raw, carried: carried.map(Box::new) })
        }
        Job::Flag { path, id, flag, on, .. } => {
            c.set_flag(path, id, *flag, *on)?;
            None
        }
        Job::Move { path, id, dest, .. } => {
            c.move_to(path, id, dest)?;
            None
        }
        Job::Delete { path, id, .. } => {
            c.delete_message(path, id)?;
            None
        }
        Job::Empty { path, .. } => {
            c.empty_folder(path)?;
            None
        }
        Job::Categories { path, id, categories, removed, .. } => {
            c.set_categories(path, id, categories, removed)?;
            None
        }
        Job::Junk { path, id, junk, .. } => {
            // Best effort: a store that keeps no such mark is no reason to retry or to complain.
            if let Err(e) = c.mark_junk(path, id, *junk) {
                log::info!("junk mark not taken: {e}");
            }
            None
        }
        Job::CreateFolder { account_id, parent, name } => {
            c.create_folder(parent.as_deref(), name)?;
            Some(Event::Folders { account_id: account_id.clone(), folders: c.folders()? })
        }
        Job::RenameFolder { account_id, path, name } => {
            c.rename_folder(path, name)?;
            Some(Event::Folders { account_id: account_id.clone(), folders: c.folders()? })
        }
        Job::DeleteFolder { account_id, path } => {
            c.delete_folder(path)?;
            Some(Event::Folders { account_id: account_id.clone(), folders: c.folders()? })
        }
        // Only reached for a store that sends itself; SMTP accounts are served in `serve`.
        Job::Send { draft_id, draft, .. } => {
            let signed = merel_smtp::build_for_store(&account.address, draft)
                .map_err(|e| describe_smtp(&e))
                .and_then(|message| outgoing(keys, account, draft, message.formatted()));
            let result = match signed {
                Ok(raw) => match c.send(&raw)? {
                    true => Ok(()),
                    false => Err("Dit account heeft geen uitgaande server.".to_string()),
                },
                Err(e) => Err(e),
            };
            Some(Event::Sent { account_id: account.id.clone(), draft_id: *draft_id, result })
        }
    }
    .into_iter()
    .collect())
}

#[cfg(test)]
mod tests {
    use merel_mailbox::{Flag, Item, ItemStore};
    use merel_model::{Address, Appointment, Auth, Contact, Server, Task, Tls};
    use merel_secrets::Secret;

    use super::*;

    /// An account whose server refuses the connection at once (nothing listens on port 1).
    fn unreachable() -> Account {
        Account {
            id: "a1".into(),
            name: "Test".into(),
            address: Address::new(None, "a@b.nl").unwrap(),
            username: "a@b.nl".into(),
            protocol: Protocol::ImapSmtp,
            auth: Auth::Password,
            incoming: Some(Server { host: "127.0.0.1".into(), port: 1, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        }
    }

    #[test]
    fn without_a_connection_changes_wait_and_reads_fail() {
        let (tx, rx) = channel();
        let mut worker = Worker::new(tx, Keys::default());
        let account = unreachable();
        worker.start_account(account.clone(), Credentials::new(&account, Secret::new("pw"), None));
        let flag = Job::Flag { account_id: "a1".into(), path: "INBOX".into(), id: "7".into(), flag: Flag::Seen, on: true };
        worker.submit(flag.clone());
        worker.submit(Job::Headers {
            account_id: "a1".into(),
            folder_id: "a1/INBOX".into(),
            path: "INBOX".into(),
            offset: 0,
            limit: crate::HEADER_LIMIT,
        });
        let wait = Duration::from_secs(20);
        match rx.recv_timeout(wait).expect("an event") {
            Event::Deferred { account_id, job } => assert_eq!((account_id.as_str(), *job), ("a1", flag)),
            other => panic!("expected the change to wait, got {other:?}"),
        }
        match rx.recv_timeout(wait).expect("an event") {
            Event::Failed { account_id, what } => assert!(account_id == "a1" && what.contains("127.0.0.1"), "{what}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    /// A store whose To Do part refuses, as Microsoft 365 did on a first live test.
    struct NoTasks;

    impl ItemStore for NoTasks {
        fn appointments(&mut self, _: i64, _: i64) -> merel_mailbox::Result<Vec<Appointment>> {
            Ok(Vec::new())
        }
        fn contacts(&mut self) -> merel_mailbox::Result<Vec<Contact>> {
            Ok(Vec::new())
        }
        fn tasks(&mut self) -> merel_mailbox::Result<Vec<Task>> {
            Err(MailboxError::Protocol("Invalid request".into()))
        }
        fn save(&mut self, _: &Item) -> merel_mailbox::Result<String> {
            Ok(String::new())
        }
        fn delete(&mut self, _: &str) -> merel_mailbox::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn refused_tasks_leave_the_calendar_standing() {
        let events = sync(&mut NoTasks, "a", 0, 1).unwrap();
        assert!(matches!(&events[0], Event::Synced { account_id, .. } if account_id == "a"));
        assert!(matches!(&events[1], Event::ItemsFailed { what, .. } if what == "Taken: De server gaf een fout: Invalid request"));
    }
}
