//! One background thread per account, holding its IMAP connection and serving [`Job`]s in order.
//!
//! The thread reconnects once when a command fails on a dropped connection; after that the
//! failure is reported and the next job tries again from scratch.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use mailcraft_imap::{ImapClient, ImapError};
use mailcraft_model::{Account, Server};
use mailcraft_secrets::Secret;

use crate::{Event, Job};

pub struct Worker {
    events: Sender<Event>,
    accounts: HashMap<String, Sender<Job>>,
}

impl Worker {
    pub fn new(events: Sender<Event>) -> Self {
        Worker { events, accounts: HashMap::new() }
    }

    /// Starts the thread for `account`. Jobs for it queue until the connection is up.
    pub fn start_account(&mut self, account: Account, password: Secret) {
        let (tx, rx) = channel();
        let events = self.events.clone();
        let id = account.id.clone();
        let name = format!("imap-{id}");
        if thread::Builder::new().name(name).spawn(move || serve(account, password, rx, events)).is_ok() {
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

fn clip(s: &str) -> String {
    mailcraft_model::clean_text(s).chars().take(160).collect()
}

fn serve(account: Account, password: Secret, jobs: Receiver<Job>, events: Sender<Event>) {
    let Some(server) = account.incoming.clone() else {
        let _ = events.send(Event::Failed { account_id: account.id, what: "Dit account heeft geen inkomende server.".into() });
        return;
    };
    let mut client: Option<ImapClient> = None;
    while let Ok(job) = jobs.recv() {
        // Connect lazily, and once more after a failure, so a dropped connection heals itself.
        let mut attempts = 0;
        loop {
            attempts += 1;
            if client.is_none() {
                match connect(&server, &account.username, &password) {
                    Ok(c) => {
                        client = Some(c);
                    }
                    Err(e) => {
                        let _ = events.send(Event::Failed { account_id: account.id.clone(), what: describe(&e) });
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

fn connect(server: &Server, username: &str, password: &Secret) -> Result<ImapClient, ImapError> {
    ImapClient::connect(server, username, password)
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
    })
}
