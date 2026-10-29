//! Signal as a mail store: the chats of a Signal account, read through the separate program
//! `merel-signal` and shown as messages in one folder, so the list, the reading pane, the chats
//! and search treat them like mail and say they came over Signal.
//!
//! `merel-signal` is AGPL (the Signal libraries are), so Merel never links it: it starts the
//! program and speaks JSON lines with it (see `apps/merel-signal`). The program keeps the
//! conversation in its own encrypted store and passes all of it on each start, then every new
//! message; this side keeps them in memory and only remembers which ones were read.
//!
//! People and groups get addresses under [`DOMAIN`], a reserved name nothing can be mailed to:
//! `<service id>@signal.invalid`, and a group's id as the chat's group id. A chat message to
//! one of them goes out over Signal instead.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use merel_mailbox::{Flag, Mailbox, MailboxError, RemoteFolder, RemoteHeader, Result};
use merel_model::FolderKind;
use merel_secrets::Secret;
use serde::{Deserialize, Serialize};

/// The domain of every Signal address. `.invalid` is reserved: mail to it goes nowhere.
pub const DOMAIN: &str = "signal.invalid";
/// The one folder a Signal account has.
pub const FOLDER: &str = "signal";
/// The environment variable that hands the store's key to the program.
const KEY_VAR: &str = "MEREL_SIGNAL_KEY";
/// How long the program may take to pass on what it stored and what waited on the server.
const FIRST_SYNC: Duration = Duration::from_secs(60);
/// How long a message may take to go out.
const SEND_WAIT: Duration = Duration::from_secs(30);
/// Longest message text kept, in bytes.
const MAX_TEXT: usize = 64 * 1024;
/// How long a call rings at most, after which it is no longer shown as ringing.
const RING_TIME: Duration = Duration::from_secs(120);
/// Most messages kept.
const MAX_MESSAGES: usize = 50_000;

/// The program, next to Merel's own (in the app bundle, or `target/` while developing), unless
/// `MEREL_SIGNAL_HELPER` names another.
pub fn helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MEREL_SIGNAL_HELPER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    let name = if cfg!(windows) { "merel-signal.exe" } else { "merel-signal" };
    std::env::current_exe().ok().map(|exe| exe.with_file_name(name)).filter(|p| p.is_file())
}

/// The address Merel gives the Signal user with service id `id`.
pub fn address_of(id: &str) -> String {
    // A PNI reads `PNI:<uuid>`; a colon has no place in an address.
    format!("{}@{DOMAIN}", id.trim().to_lowercase().replace(':', "."))
}

/// Whether `email` is a Signal address.
pub fn is_signal_address(email: &str) -> bool {
    email.rsplit_once('@').is_some_and(|(_, domain)| domain.eq_ignore_ascii_case(DOMAIN))
}

/// What the program says, one JSON object per line.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Said {
    Link { url: String },
    Linked { number: Option<String>, id: String },
    Message(Received),
    Synced,
    Sent { thread: String, timestamp: u64 },
    Error { message: String },
}

/// A message as the program passes it on.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Received {
    pub thread: String,
    #[serde(default)]
    pub thread_name: String,
    #[serde(default)]
    pub group: bool,
    pub sender: String,
    #[serde(default)]
    pub sender_name: String,
    #[serde(default)]
    pub mine: bool,
    pub timestamp: u64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<String>,
    /// Set when this is a call rather than a message.
    #[serde(default)]
    pub call: Option<Call>,
}

/// A Signal call in a chat. Merel cannot take part in it: it shows who calls and how it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Call {
    /// Signal's id of the call: news about the same call replaces what was shown before.
    pub id: u64,
    pub kind: CallKind,
    pub state: CallState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Audio,
    Video,
    Group,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    Ringing,
    Answered,
    Missed,
    Started,
}

impl Call {
    /// What the chat shows for it, in the person's language.
    pub fn text(&self, mine: bool) -> String {
        let key = match (self.kind, self.state, mine) {
            (CallKind::Group, _, _) => "signal-call-group",
            (_, CallState::Ringing, _) => "signal-call-ringing",
            (CallKind::Video, CallState::Missed, false) => "signal-call-missed-video",
            (_, CallState::Missed, false) => "signal-call-missed",
            (CallKind::Video, _, true) => "signal-call-out-video",
            (_, _, true) => "signal-call-out",
            (CallKind::Video, _, false) => "signal-call-in-video",
            (_, _, false) => "signal-call-in",
        };
        format!("📞 {}", merel_i18n::tr(key))
    }

    /// A call that needs no attention: answered, made by the account, or a group call.
    fn settled(&self, mine: bool) -> bool {
        mine || matches!(self.state, CallState::Answered | CallState::Started)
    }
}

impl Received {
    /// The id Merel lists it under: unique, as Signal makes a sender's timestamps unique. A
    /// call keeps one id from ringing to its end, so its entry changes instead of adding up.
    pub fn id(&self) -> String {
        match self.call {
            Some(call) if call.kind != CallKind::Group => format!("call.{}", call.id),
            _ => format!("{}.{}", self.timestamp, self.sender.to_lowercase().replace(':', ".")),
        }
    }

    /// The text to show: the message, or what happened with the call.
    fn shown_text(&self) -> String {
        match self.call {
            // No news of how it ended (it may have been answered on the phone): just "a call".
            Some(mut call) if call.state == CallState::Ringing && self.age() > RING_TIME => {
                call.state = CallState::Answered;
                call.text(self.mine)
            }
            Some(call) => call.text(self.mine),
            None => self.text.clone(),
        }
    }

    fn age(&self) -> Duration {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        now.saturating_sub(Duration::from_millis(self.timestamp))
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Asked<'a> {
    Send { thread: &'a str, text: &'a str },
}

/// The program running, and what it said so far.
struct Program {
    child: Child,
    said: Receiver<Said>,
}

impl Program {
    fn start(helper: &Path, args: &[&std::ffi::OsStr], key: &Secret) -> std::io::Result<(Program, Option<ChildStdin>)> {
        let mut child =
            Command::new(helper).args(args).env(KEY_VAR, key.expose()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no output"))?;
        let (tx, rx) = channel();
        std::thread::Builder::new().name("signal".into()).spawn(move || read(stdout, &tx))?;
        // What the program notes along the way (never message text) goes to Merel's log.
        if let Some(stderr) = child.stderr.take() {
            std::thread::Builder::new().name("signal-log".into()).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
                    log::warn!("{}", clean(&line));
                }
            })?;
        }
        Ok((Program { child, said: rx }, stdin))
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read(out: impl std::io::Read, tx: &Sender<Said>) {
    for line in BufReader::new(out).lines() {
        let Ok(line) = line else { break };
        match serde_json::from_str::<Said>(&line) {
            Ok(said) => {
                if tx.send(said).is_err() {
                    break;
                }
            }
            Err(e) => log::warn!("signal: a line Merel does not understand: {e}"),
        }
    }
}

/// Linking Merel to a Signal account, as Signal Desktop does: the program gives a link, the
/// Signal app on the phone scans it as a QR code, and the account is linked.
pub struct Linking {
    program: Program,
}

/// How far linking has come.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkStep {
    /// Show this as a QR code.
    Scan(String),
    /// Linked to the account with this phone number and service id.
    Linked {
        number: Option<String>,
        id: String,
    },
    Failed(String),
}

impl Linking {
    /// Starts linking a new store in `dir`, encrypted with `key`; the phone lists the computer
    /// as `device`.
    pub fn start(helper: &Path, dir: &Path, key: &Secret, device: &str) -> std::result::Result<Linking, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let args = [std::ffi::OsStr::new("link"), dir.as_os_str(), std::ffi::OsStr::new(device)];
        let (program, _) = Program::start(helper, &args, key).map_err(|e| e.to_string())?;
        Ok(Linking { program })
    }

    /// What happened since the last call, if anything.
    pub fn poll(&mut self) -> Option<LinkStep> {
        match self.program.said.try_recv() {
            Ok(Said::Link { url }) => Some(LinkStep::Scan(url)),
            Ok(Said::Linked { number, id }) => Some(LinkStep::Linked { number, id }),
            Ok(Said::Error { message }) => Some(LinkStep::Failed(message)),
            Ok(_) => None,
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(LinkStep::Failed(merel_i18n::tr("signal-stopped"))),
        }
    }
}

/// The messages of a Signal account, as far as they came in.
#[derive(Default)]
struct Messages {
    by_id: HashMap<String, Received>,
    /// Who each Signal address is, for sending: the address to the program's thread.
    threads: HashMap<String, String>,
    /// The latest name each person was given, by service id: a name may come only with a later
    /// message (once the phone shared its contacts), and then goes for the earlier ones too.
    names: HashMap<String, String>,
}

impl Messages {
    fn add(&mut self, m: Received) {
        if self.by_id.len() >= MAX_MESSAGES && !self.by_id.contains_key(&m.id()) {
            return;
        }
        if !m.group {
            self.threads.insert(address_of(&m.thread), m.thread.clone());
            if is_name(&m.thread_name, &m.thread) {
                self.names.insert(m.thread.clone(), m.thread_name.clone());
            }
        }
        if !m.mine && is_name(&m.sender_name, &m.sender) {
            self.names.insert(m.sender.clone(), m.sender_name.clone());
        }
        // News of a call can come in any order (the store passes the newest first): once any
        // device says it was answered, it was.
        if let (Some(new), Some(Received { call: Some(old), .. })) = (m.call, self.by_id.get(&m.id()))
            && old.state == CallState::Answered
            && new.state != CallState::Answered
        {
            return;
        }
        let mut m = m;
        if m.text.len() > MAX_TEXT {
            let mut cut = MAX_TEXT;
            while !m.text.is_char_boundary(cut) {
                cut -= 1;
            }
            m.text.truncate(cut);
        }
        self.by_id.insert(m.id(), m);
    }
}

impl Messages {
    /// `m` with the best names known now: the account's own for what it sent.
    fn named(&self, m: &Received, me: &str) -> Received {
        let name = |id: &str, given: &str| match self.names.get(id) {
            Some(known) if !is_name(given, id) => known.clone(),
            _ => given.to_string(),
        };
        let mut m = m.clone();
        m.sender_name = if m.mine { me.to_string() } else { name(&m.sender, &m.sender_name) };
        if !m.group {
            m.thread_name = name(&m.thread, &m.thread_name);
        }
        m
    }
}

/// Whether `name` is a name, not empty or the service id it stands for.
fn is_name(name: &str, id: &str) -> bool {
    !name.trim().is_empty() && !name.eq_ignore_ascii_case(id)
}

/// A linked Signal account, read through the program.
pub struct SignalMailbox {
    program: Program,
    stdin: ChildStdin,
    messages: Arc<Mutex<Messages>>,
    /// What came from the program besides messages, for [`SignalMailbox::send`].
    replies: Receiver<Said>,
    /// The account's own address and name.
    me: String,
    me_name: String,
    flags: Flags,
    /// Set when the program stopped, with why: the next listing fails, so Merel connects again.
    stopped: Arc<Mutex<Option<String>>>,
}

impl SignalMailbox {
    /// Starts the program on the store in `dir` and waits until it passed on everything it has.
    pub fn connect(helper: &Path, dir: &Path, key: &Secret, me: &merel_model::Address) -> Result<SignalMailbox> {
        let unreachable = |reason: String| MailboxError::Connect { host: "Signal".into(), reason };
        let args = [std::ffi::OsStr::new("run"), dir.as_os_str()];
        let (mut program, stdin) = Program::start(helper, &args, key).map_err(|e| unreachable(e.to_string()))?;
        let stdin = stdin.ok_or_else(|| unreachable("no input".into()))?;
        let messages = Arc::new(Mutex::new(Messages::default()));
        let deadline = Instant::now() + FIRST_SYNC;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match program.said.recv_timeout(left) {
                Ok(Said::Message(m)) => lock(&messages).add(m),
                Ok(Said::Synced) => break,
                Ok(Said::Error { message }) => return Err(unreachable(message)),
                Ok(_) => {}
                // Slow, not broken: what came so far is shown, the rest follows.
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Err(unreachable(merel_i18n::tr("signal-stopped"))),
            }
        }
        // From here on a thread takes in the messages as they come; the rest is for `send`.
        let (tx, replies) = channel();
        let said = std::mem::replace(&mut program.said, channel().1);
        let into = messages.clone();
        let stopped = Arc::new(Mutex::new(None));
        let stop = stopped.clone();
        std::thread::Builder::new()
            .name("signal-in".into())
            .spawn(move || {
                let mut last_error = None;
                for s in said {
                    match s {
                        Said::Message(m) => lock(&into).add(m),
                        other => {
                            if let Said::Error { message } = &other {
                                log::warn!("signal: {}", clean(message));
                                last_error = Some(message.clone());
                            }
                            // Nobody waits for a reply between sends; that is fine.
                            let _ = tx.send(other);
                        }
                    }
                }
                *stop.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(last_error.unwrap_or_else(|| merel_i18n::tr("signal-stopped")));
            })
            .map_err(|e| unreachable(e.to_string()))?;
        Ok(SignalMailbox {
            program,
            stdin,
            messages,
            replies,
            me: me.email().to_string(),
            me_name: me.display().to_string(),
            flags: Flags::open(dir),
            stopped,
        })
    }

    fn header(&self, messages: &Messages, m: &Received) -> RemoteHeader {
        let raw = message(&messages.named(m, &self.me_name), &self.me);
        let id = m.id();
        RemoteHeader {
            seen: m.mine || m.call.is_some_and(|c| c.settled(m.mine)) || self.flags.has(Flag::Seen, &id),
            flagged: self.flags.has(Flag::Flagged, &id),
            answered: false,
            size: u32::try_from(raw.len()).unwrap_or(u32::MAX),
            has_attachments: !m.attachments.is_empty(),
            header: merel_mime::parse_header(&raw),
            date: i64::try_from(m.timestamp / 1000).unwrap_or_default(),
            categories: Vec::new(),
            id,
        }
    }

    fn listed(&self, keep: impl Fn(&Received) -> bool, offset: u32, limit: u32) -> Vec<RemoteHeader> {
        let messages = lock(&self.messages);
        let mut all: Vec<&Received> = messages.by_id.values().filter(|m| keep(m)).collect();
        all.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| a.sender.cmp(&b.sender)));
        all.into_iter().skip(offset as usize).take(limit as usize).map(|m| self.header(&messages, m)).collect()
    }
}

fn lock(m: &Mutex<Messages>) -> std::sync::MutexGuard<'_, Messages> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn only_folder(folder: &str) -> Result<()> {
    if folder == FOLDER { Ok(()) } else { Err(MailboxError::NotFound(folder.to_string())) }
}

fn not_here() -> MailboxError {
    MailboxError::Protocol(merel_i18n::tr("signal-cannot"))
}

impl Mailbox for SignalMailbox {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        Ok(vec![RemoteFolder { path: FOLDER.into(), name: "Signal".into(), kind: FolderKind::Inbox, selectable: true, parent: None }])
    }

    fn headers(&mut self, folder: &str, offset: u32, limit: u32) -> Result<Vec<RemoteHeader>> {
        only_folder(folder)?;
        if let Some(why) = self.stopped.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone() {
            return Err(MailboxError::Connect { host: "Signal".into(), reason: why });
        }
        Ok(self.listed(|_| true, offset, limit))
    }

    fn search(&mut self, folder: &str, query: &str, limit: u32) -> Result<Vec<RemoteHeader>> {
        only_folder(folder)?;
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let found = |m: &Received| {
            let text = format!("{} {} {}", m.text, m.sender_name, m.thread_name).to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
        };
        Ok(self.listed(found, 0, limit))
    }

    fn body(&mut self, folder: &str, id: &str) -> Result<Vec<u8>> {
        only_folder(folder)?;
        let messages = lock(&self.messages);
        let m = messages.by_id.get(id).ok_or_else(|| MailboxError::NotFound(id.to_string()))?;
        Ok(message(&messages.named(m, &self.me_name), &self.me))
    }

    fn set_flag(&mut self, folder: &str, id: &str, flag: Flag, on: bool) -> Result<()> {
        only_folder(folder)?;
        self.flags.set(flag, id, on);
        Ok(())
    }

    fn move_to(&mut self, _folder: &str, _id: &str, _dest: &str) -> Result<()> {
        Err(not_here())
    }

    fn append(&mut self, _folder: &str, _message: &[u8]) -> Result<()> {
        Err(not_here())
    }

    /// Sends the text of a chat message over Signal, to the group it names or the one person
    /// it is to.
    fn send(&mut self, raw: &[u8]) -> Result<bool> {
        let header = merel_mime::parse_header(raw);
        let thread = match header.chat.as_ref().and_then(|c| c.group_id.clone()) {
            Some(group) => group,
            None => {
                let [to] = header.to.as_slice() else { return Err(MailboxError::Protocol(merel_i18n::tr("signal-one-person"))) };
                let threads = &lock(&self.messages).threads;
                threads.get(&to.email().to_lowercase()).cloned().ok_or_else(|| MailboxError::Protocol(merel_i18n::tr("signal-unknown")))?
            }
        };
        let text = merel_mime::parse_body(raw).text.trim_end().to_string();
        let line = serde_json::to_string(&Asked::Send { thread: &thread, text: &text }).map_err(|e| MailboxError::Protocol(e.to_string()))?;
        writeln!(self.stdin, "{line}").and_then(|()| self.stdin.flush()).map_err(|_| MailboxError::Protocol(merel_i18n::tr("signal-stopped")))?;
        let deadline = Instant::now() + SEND_WAIT;
        loop {
            match self.replies.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Said::Sent { thread: t, .. }) if t == thread => return Ok(true),
                Ok(Said::Error { message }) => return Err(MailboxError::Protocol(message)),
                Ok(_) => {}
                Err(_) => return Err(MailboxError::Protocol(merel_i18n::tr("signal-no-answer"))),
            }
        }
    }

    fn create_folder(&mut self, _parent: Option<&str>, _name: &str) -> Result<()> {
        Err(not_here())
    }

    fn rename_folder(&mut self, _folder: &str, _name: &str) -> Result<()> {
        Err(not_here())
    }

    fn delete_folder(&mut self, _folder: &str) -> Result<()> {
        Err(not_here())
    }

    fn delete_message(&mut self, _folder: &str, _id: &str) -> Result<()> {
        Err(not_here())
    }

    fn empty_folder(&mut self, _folder: &str) -> Result<()> {
        Err(not_here())
    }

    fn set_categories(&mut self, _folder: &str, _id: &str, _categories: &[String], _removed: &[String]) -> Result<()> {
        Err(not_here())
    }

    fn logout(mut self: Box<Self>) {
        self.program.stop();
    }
}

/// The message as mail: the text as a plain part, from the sender to the account, with the
/// chat headers that put it in its chat.
fn message(m: &Received, me: &str) -> Vec<u8> {
    let from = if m.mine { me.to_string() } else { address_of(&m.sender) };
    let to = if m.mine { thread_address(m) } else { me.to_string() };
    let from_name = if m.sender_name.is_empty() { &m.sender } else { &m.sender_name };
    let date = chrono::DateTime::from_timestamp_millis(i64::try_from(m.timestamp).unwrap_or_default()).unwrap_or_default().to_rfc2822();
    let mut text = m.shown_text();
    for name in &m.attachments {
        text.push_str(&format!("\n[{}: {}]", merel_i18n::tr("signal-attachment"), clean(name)));
    }
    let mut out = String::new();
    out.push_str(&format!("From: {} <{from}>\r\n", word(from_name)));
    if m.group || !m.mine {
        out.push_str(&format!("To: <{to}>\r\n"));
    } else {
        out.push_str(&format!("To: {} <{to}>\r\n", word(&m.thread_name)));
    }
    out.push_str(&format!("Subject: {}\r\n", word(&merel_mime::preview(&m.shown_text(), 80))));
    out.push_str(&format!("Date: {date}\r\n"));
    out.push_str(&format!("Message-ID: <{}@{DOMAIN}>\r\n", m.id()));
    out.push_str("Chat-Version: 1.0\r\n");
    if m.group {
        out.push_str(&format!("Chat-Group-ID: {}\r\n", m.thread));
        if !m.thread_name.is_empty() {
            out.push_str(&format!("Chat-Group-Name: {}\r\n", word(&m.thread_name)));
        }
    }
    out.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n");
    let body = base64::engine::general_purpose::STANDARD.encode(text.replace('\n', "\r\n"));
    for chunk in body.as_bytes().chunks(76) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

/// Where a message sent in a chat between two people went: the other person.
fn thread_address(m: &Received) -> String {
    if m.group { format!("{}@{DOMAIN}", m.thread) } else { address_of(&m.thread) }
}

/// Names and file names without line breaks or other control characters.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// `s` as an encoded word, so any name or text fits in a header.
fn word(s: &str) -> String {
    format!("=?utf-8?b?{}?=", base64::engine::general_purpose::STANDARD.encode(clean(s)))
}

/// Which messages were read or flagged, kept in a file next to the store. Only hashes of the
/// ids go there, so the file says nothing about who wrote.
struct Flags {
    path: Option<PathBuf>,
    on: HashSet<String>,
}

impl Flags {
    fn open(dir: &Path) -> Flags {
        let path = dir.join("merel-flags.json");
        let on = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Flags { path: Some(path), on }
    }

    fn key(flag: Flag, id: &str) -> String {
        use sha2::Digest as _;
        let tag = match flag {
            Flag::Seen => "seen",
            Flag::Flagged => "flagged",
        };
        let hash = sha2::Sha256::digest(format!("{tag}:{id}").as_bytes());
        hash[..12].iter().map(|b| format!("{b:02x}")).collect()
    }

    fn has(&self, flag: Flag, id: &str) -> bool {
        self.on.contains(&Flags::key(flag, id))
    }

    fn set(&mut self, flag: Flag, id: &str, on: bool) {
        let key = Flags::key(flag, id);
        let changed = if on { self.on.insert(key) } else { self.on.remove(&key) };
        if changed
            && let Some(path) = &self.path
            && let Ok(bytes) = serde_json::to_vec(&self.on)
            && let Err(e) = std::fs::write(path, bytes)
        {
            log::warn!("signal: could not keep what was read: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received(text: &str) -> Received {
        Received {
            thread: "1b2c".into(),
            thread_name: "Jan".into(),
            group: false,
            sender: "1b2c".into(),
            sender_name: "Jan de Vries".into(),
            mine: false,
            timestamp: 1_761_681_600_123,
            text: text.into(),
            attachments: Vec::new(),
            call: None,
        }
    }

    #[test]
    fn a_message_reads_as_a_chat_message_from_the_sender() {
        let raw = message(&received("Hoi, kom je morgen ook?\nGroet"), "me@signal.invalid");
        let header = merel_mime::parse_header(&raw);
        assert_eq!(header.from.as_ref().map(|a| a.email().to_string()).as_deref(), Some("1b2c@signal.invalid"));
        assert_eq!(header.from.as_ref().map(|a| a.display().to_string()).as_deref(), Some("Jan de Vries"));
        assert_eq!(header.date, Some(1_761_681_600));
        assert!(header.chat.is_some_and(|c| c.group_id.is_none()));
        assert_eq!(merel_mime::parse_body(&raw).text.replace("\r\n", "\n").trim_end(), "Hoi, kom je morgen ook?\nGroet");
    }

    #[test]
    fn a_group_message_carries_the_group() {
        let mut m = received("Wie neemt de taart mee?");
        m.group = true;
        m.thread = "g0123abcd".into();
        m.thread_name = "Familie".into();
        let header = merel_mime::parse_header(&message(&m, "me@signal.invalid"));
        let chat = header.chat.unwrap_or_default();
        assert_eq!(chat.group_id.as_deref(), Some("g0123abcd"));
        assert_eq!(chat.group_name.as_deref(), Some("Familie"));
    }

    #[test]
    fn a_name_cannot_add_a_header() {
        let mut m = received("x");
        m.sender_name = "Jan\r\nBcc: a@b.nl".into();
        let raw = String::from_utf8(message(&m, "me@signal.invalid")).unwrap_or_default();
        assert!(!raw.lines().any(|l| l.starts_with("Bcc")));
    }

    #[test]
    fn a_name_that_comes_later_names_the_earlier_messages_too() {
        let mut messages = Messages::default();
        let mut first = received("Heb ik de borrel gemist?");
        first.mine = true;
        first.sender = "me".into();
        first.sender_name = "me".into();
        first.thread_name = "1b2c".into();
        messages.add(first.clone());
        let mut answer = received("Die komt nog.");
        answer.sender_name = "Maarten".into();
        answer.thread_name = "1b2c".into();
        answer.timestamp += 1;
        messages.add(answer);
        let named = messages.named(&first, "+31612345678");
        assert_eq!(named.thread_name, "Maarten");
        assert_eq!(named.sender_name, "+31612345678");
        let header = merel_mime::parse_header(&message(&named, "me@signal.invalid"));
        assert_eq!(header.to.first().map(|a| a.display().to_string()).as_deref(), Some("Maarten"));
    }

    #[test]
    fn what_the_program_says_is_read() {
        let line = r#"{"type":"message","thread":"1b2c","thread_name":"Jan","group":false,"sender":"1b2c","sender_name":"Jan","mine":false,"timestamp":5,"text":"hoi","attachments":["foto.jpg"]}"#;
        let Ok(Said::Message(m)) = serde_json::from_str::<Said>(line) else { panic!("not read") };
        assert_eq!(m.attachments, vec!["foto.jpg".to_string()]);
        assert_eq!(m.id(), "5.1b2c");
        assert_eq!(serde_json::from_str::<Said>(r#"{"type":"synced"}"#).ok(), Some(Said::Synced));
    }

    #[test]
    fn a_call_keeps_one_entry_from_ringing_to_its_end() {
        let line = |state: &str, ts: u64| {
            format!(
                r#"{{"type":"message","thread":"1b2c","sender":"1b2c","timestamp":{ts},"text":"","call":{{"id":77,"kind":"audio","state":"{state}"}}}}"#
            )
        };
        let mut messages = Messages::default();
        for (state, ts) in [("ringing", 5), ("missed", 9)] {
            let Ok(Said::Message(m)) = serde_json::from_str::<Said>(&line(state, ts)) else { panic!("not read") };
            messages.add(m);
        }
        assert_eq!(messages.by_id.len(), 1);
        let m = messages.by_id.values().next().cloned().unwrap_or_else(|| received(""));
        assert_eq!(m.call.map(|c| c.state), Some(CallState::Missed));
        let raw = message(&m, "me@signal.invalid");
        let text = merel_mime::parse_body(&raw).text;
        assert!(text.starts_with("📞 ") && text.trim().len() > "📞 ".len(), "{text}");
        for (state, ts) in [("answered", 6), ("missed", 9)] {
            let Ok(Said::Message(m)) = serde_json::from_str::<Said>(&line(state, ts)) else { panic!("not read") };
            messages.add(m);
        }
        assert_eq!(messages.by_id.values().next().and_then(|m| m.call).map(|c| c.state), Some(CallState::Answered));
    }

    #[test]
    fn a_missed_call_is_unread_and_an_answered_one_is_not() {
        let call = |state| Call { id: 1, kind: CallKind::Video, state };
        assert!(!call(CallState::Missed).settled(false));
        assert!(!call(CallState::Ringing).settled(false));
        assert!(call(CallState::Answered).settled(false));
        assert!(call(CallState::Missed).settled(true));
    }

    #[test]
    fn addresses_are_under_the_reserved_domain() {
        assert_eq!(address_of("PNI:AB-12"), "pni.ab-12@signal.invalid");
        assert!(is_signal_address("x@Signal.Invalid"));
        assert!(!is_signal_address("x@gmail.com"));
    }

    #[test]
    fn flags_keep_only_hashes() {
        let dir = std::env::temp_dir().join(format!("merel-signal-flags-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut flags = Flags::open(&dir);
        flags.set(Flag::Seen, "5.1b2c", true);
        let kept = std::fs::read_to_string(dir.join("merel-flags.json")).unwrap_or_default();
        assert!(!kept.contains("1b2c"));
        assert!(Flags::open(&dir).has(Flag::Seen, "5.1b2c"));
        assert!(!Flags::open(&dir).has(Flag::Flagged, "5.1b2c"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
