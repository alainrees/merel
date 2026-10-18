//! One background thread per account, holding its IMAP connection and serving [`Job`]s in order.
//!
//! The thread reconnects once when a command fails on a dropped connection; after that the
//! failure is reported and the next job tries again from scratch.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use mailcraft_imap::{ImapClient, ImapError};
use mailcraft_model::{Account, Server};
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
        let name = format!("imap-{id}");
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

fn describe(e: &ImapError) -> String {
    match e {
        ImapError::Connect { host, .. } => format!("Kan geen verbinding maken met {host}."),
        ImapError::Login => "De server accepteert de gebruikersnaam of het wachtwoord niet.".into(),
        ImapError::Protocol(e) => format!("De server gaf een fout: {}", clip(&e.to_string())),
        ImapError::BadName(_) => "Een mapnaam is niet toegestaan.".into(),
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

fn serve(account: Account, mut credentials: Credentials, jobs: Receiver<Job>, events: Sender<Event>) {
    let Some(server) = account.incoming.clone() else {
        let _ = events.send(Event::Failed { account_id: account.id, what: "Dit account heeft geen inkomende server.".into() });
        return;
    };
    let mut client: Option<ImapClient> = None;
    while let Ok(job) = jobs.recv() {
        if let Job::Send { draft_id, draft, sent_path, .. } = &job {
            let result = send(&account, &mut credentials, draft).map(|raw| {
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
                match connect(&server, &account.username, &mut credentials) {
                    Ok(c) => {
                        client = Some(c);
                    }
                    Err(what) => {
                        // A refused login may be a stale access token: fetch a new one next time.
                        credentials.invalidate();
                        let _ = events.send(Event::Failed { account_id: account.id.clone(), what });
                        break;
                    }
                }
            }
            let Some(c) = client.as_mut() else { break };
            match run(c, &job) {
                Ok(Some(ev)) => {
                    let _ = events.send(ev);
                    break;
                }
                Ok(None) => break,
                Err(e) => {
                    log::warn!("{}: {} (attempt {attempts})", account.id, e);
                    client = None;
                    if attempts >= 2 {
                        let _ = events.send(Event::Failed { account_id: account.id.clone(), what: describe(&e) });
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

/// Builds and sends `draft`; returns the bytes that went out, for the Sent folder.
fn send(account: &Account, credentials: &mut Credentials, draft: &mailcraft_model::Draft) -> Result<Vec<u8>, String> {
    let server = account.outgoing.as_ref().ok_or_else(|| "Dit account heeft geen uitgaande server.".to_string())?;
    let message = mailcraft_smtp::build(&account.address, draft).map_err(|e| describe_smtp(&e))?;
    let credential = credentials.smtp()?;
    mailcraft_smtp::send(server, &account.username, &credential, &message).map_err(|e| describe_smtp(&e))?;
    Ok(message.formatted())
}

fn connect(server: &Server, username: &str, credentials: &mut Credentials) -> Result<ImapClient, String> {
    let credential = credentials.imap()?;
    ImapClient::connect(server, username, &credential).map_err(|e| describe(&e))
}

fn run(c: &mut ImapClient, job: &Job) -> Result<Option<Event>, ImapError> {
    Ok(match job {
        Job::Connect { account_id } => Some(Event::Connected { account_id: account_id.clone(), folders: c.folders()? }),
        Job::Headers { folder_id, path, .. } => Some(Event::Headers { folder_id: folder_id.clone(), headers: c.headers(path, crate::HEADER_LIMIT)? }),
        Job::Body { message_id, path, uid, .. } => {
            let raw = c.body(path, *uid)?;
            Some(Event::Body { message_id: message_id.clone(), body: mailcraft_mime::parse_body(&raw) })
        }
        Job::Flag { path, uid, flag, on, .. } => {
            c.set_flag(path, *uid, flag, *on)?;
            None
        }
        Job::Move { path, uid, dest, .. } => {
            c.move_to(path, *uid, dest)?;
            None
        }
        // Handled in `serve` before the IMAP connection is touched.
        Job::Send { .. } => None,
    })
}
