//! One background thread per account, holding its connection and serving [`Job`]s in order.
//!
//! The thread speaks IMAP or Exchange Web Services through the [`Mailbox`] trait and never
//! knows which. It reconnects once when a command fails on a dropped connection; after that
//! the failure is reported and the next job tries again from scratch.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use mailcraft_exchange::EwsClient;
use mailcraft_imap::ImapClient;
use mailcraft_mailbox::{Mailbox, MailboxError};
use mailcraft_model::{Account, Protocol, Server};
use mailcraft_smtp::SmtpError;

use crate::signin::Credentials;
use crate::{Event, Job};

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
        _ => Event::Failed { account_id: account_id.to_string(), what },
    }
}

fn serve(account: Account, mut credentials: Credentials, jobs: Receiver<Job>, events: Sender<Event>) {
    let Some(server) = account.incoming.clone() else {
        let _ = events.send(Event::Failed { account_id: account.id, what: "Dit account heeft geen inkomende server.".into() });
        return;
    };
    let mut client: Option<Box<dyn Mailbox>> = None;
    while let Ok(job) = jobs.recv() {
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
                        let _ = events.send(failure(&account.id, &job, what));
                        break;
                    }
                }
            }
            let Some(c) = client.as_mut() else { break };
            match run(c.as_mut(), &account, &job) {
                Ok(Some(ev)) => {
                    let _ = events.send(ev);
                    break;
                }
                Ok(None) => break,
                Err(e) => {
                    log::warn!("{}: {} (attempt {attempts})", account.id, e);
                    client = None;
                    if attempts >= 2 {
                        let _ = events.send(failure(&account.id, &job, describe(&e)));
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
        Protocol::ImapSmtp | Protocol::Gmail | Protocol::Microsoft365 => {
            Box::new(ImapClient::connect(server, &account.username, &credential).map_err(MailboxError::from).map_err(|e| describe(&e))?)
        }
    })
}

/// The item store of `c`; an error for a store that keeps only mail, which the engine never
/// asks.
fn items_of(c: &mut dyn Mailbox) -> Result<&mut dyn mailcraft_mailbox::ItemStore, MailboxError> {
    c.items().ok_or_else(|| MailboxError::Protocol("this account keeps no appointments, contacts or tasks".into()))
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
        Job::Headers { folder_id, path, .. } => Some(Event::Headers { folder_id: folder_id.clone(), headers: c.headers(path, crate::HEADER_LIMIT)? }),
        Job::Body { message_id, path, id, .. } => {
            let raw = c.body(path, id)?;
            Some(Event::Body { message_id: message_id.clone(), body: mailcraft_mime::parse_body(&raw), header: mailcraft_mime::parse_header(&raw) })
        }
        Job::Flag { path, id, flag, on, .. } => {
            c.set_flag(path, id, *flag, *on)?;
            None
        }
        Job::Move { path, id, dest, .. } => {
            c.move_to(path, id, dest)?;
            None
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
