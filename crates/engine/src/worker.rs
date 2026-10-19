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

use mailcraft_dav::DavMailbox;
use mailcraft_exchange::EwsClient;
use mailcraft_google::GoogleMailbox;
use mailcraft_graph::GraphClient;
use mailcraft_imap::ImapClient;
use mailcraft_mailbox::{Mailbox, MailboxError};
use mailcraft_model::{Account, Protocol, Server};
use mailcraft_smtp::SmtpError;

use crate::signin::Credentials;
use crate::{Event, Job};

/// First wait before trying the server again after a failed connection; it doubles each time.
const FIRST_RETRY: Duration = Duration::from_secs(15);
/// Longest wait between two tries.
const MAX_RETRY: Duration = Duration::from_secs(5 * 60);

pub struct Worker {
    events: Sender<Event>,
    accounts: HashMap<String, Sender<Job>>,
}

impl Worker {
    pub fn new(events: Sender<Event>) -> Self {
        Worker { events, accounts: HashMap::new() }
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
        if thread::Builder::new().name(name).spawn(move || serve(account, credentials, rx, events)).is_ok() {
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
        MailboxError::Connect { host, .. } => format!("Kan geen verbinding maken met {host}."),
        MailboxError::Login => "De server accepteert de gebruikersnaam of het wachtwoord niet.".into(),
        MailboxError::AuthScheme(scheme) => format!(
            "De server eist een aanmeldmethode die Mailcraft nog niet ondersteunt ({scheme}). Vraag de beheerder om Basic-verificatie voor EWS, of gebruik een Microsoft 365-account."
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
    mailcraft_model::clean_text(s).chars().take(160).collect()
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

fn longer(wait: Option<Duration>) -> Option<Duration> {
    Some(wait.map_or(FIRST_RETRY, |w| (w * 2).min(MAX_RETRY)))
}

fn serve(account: Account, mut credentials: Credentials, jobs: Receiver<Job>, events: Sender<Event>) {
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
                        Ok(mut c) => match run(c.as_mut(), &account, &Job::Connect { account_id: account.id.clone() }) {
                            Ok(ev) => {
                                if let Some(ev) = ev {
                                    let _ = events.send(ev);
                                }
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
            let result = send_smtp(&account, &mut credentials, draft).map(|raw| {
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
            match run(c.as_mut(), &account, &job) {
                Ok(Some(ev)) => {
                    retry = None;
                    let _ = events.send(ev);
                    break;
                }
                Ok(None) => {
                    retry = None;
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

/// Builds and sends `draft` over SMTP; returns the bytes that went out, for the Sent folder.
fn send_smtp(account: &Account, credentials: &mut Credentials, draft: &mailcraft_model::Draft) -> Result<Vec<u8>, String> {
    let server = account.outgoing.as_ref().ok_or_else(|| "Dit account heeft geen uitgaande server.".to_string())?;
    let message = mailcraft_smtp::build(&account.address, draft).map_err(|e| describe_smtp(&e))?;
    let credential = credentials.smtp()?;
    mailcraft_smtp::send(server, &account.username, &credential, &message).map_err(|e| describe_smtp(&e))?;
    Ok(message.formatted())
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
fn items_of(c: &mut dyn Mailbox) -> Result<&mut dyn mailcraft_mailbox::ItemStore, MailboxError> {
    c.items().ok_or_else(|| MailboxError::Protocol("this account keeps no appointments, contacts or tasks".into()))
}

/// Whether the account's sign-in covers Microsoft Graph (an account signed in before Graph
/// came along has IMAP scopes, and keeps working as mail only until it signs in again).
fn uses_graph(account: &Account) -> bool {
    matches!(&account.auth, mailcraft_model::Auth::OAuth2 { scope, .. } if scope.contains("graph.microsoft.com"))
}

fn run(c: &mut dyn Mailbox, account: &Account, job: &Job) -> Result<Option<Event>, MailboxError> {
    Ok(match job {
        Job::Connect { account_id } => {
            let folders = c.folders()?;
            Some(Event::Connected { account_id: account_id.clone(), folders, syncs_items: c.items().is_some() })
        }
        Job::Sync { account_id, from, to } => {
            let store = items_of(c)?;
            let appointments = store.appointments(*from, *to)?;
            let contacts = store.contacts()?;
            let tasks = store.tasks()?;
            Some(Event::Synced { account_id: account_id.clone(), appointments, contacts, tasks })
        }
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
        Job::Body { message_id, path, id, .. } => {
            let raw = c.body(path, id)?;
            let (body, header) = (Box::new(mailcraft_mime::parse_body(&raw)), mailcraft_mime::parse_header(&raw));
            Some(Event::Body { message_id: message_id.clone(), body, header, raw })
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
            let result = match mailcraft_smtp::build(&account.address, draft) {
                Ok(message) => match c.send(&message.formatted())? {
                    true => Ok(()),
                    false => Err("Dit account heeft geen uitgaande server.".to_string()),
                },
                Err(e) => Err(describe_smtp(&e)),
            };
            Some(Event::Sent { account_id: account.id.clone(), draft_id: *draft_id, result })
        }
    })
}

#[cfg(test)]
mod tests {
    use mailcraft_mailbox::Flag;
    use mailcraft_model::{Address, Auth, Server, Tls};
    use mailcraft_secrets::Secret;

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
        let mut worker = Worker::new(tx);
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
}
