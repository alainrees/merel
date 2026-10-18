# Mailcraft architecture

| Layer | Crate | Job |
|---|---|---|
| L1 | `secrets` | Passwords in the system credential store (Keychain, Credential Manager, Secret Service) through `keyring`; a memory store for tests. Secrets are zeroed on drop and hidden from `Debug`. |
| L1 | `model` | Accounts, folders, message headers, addresses, calendars, appointments, contacts and tasks. Validates everything that comes from the network or the keyboard (address shape, header length, control characters, appointment times). No I/O. |
| L2 | `mime` | Header and body parsing through `mail-parser`; HTML is reduced to plain text with `html2text`, so no script, style or remote reference survives. Input is bounded at 8 MB. |
| L2 | `imap` | Blocking IMAP client over rustls (implicit TLS or STARTTLS, platform certificate verifier, no way to disable checks): folders with SPECIAL-USE, newest headers with BODYSTRUCTURE, `BODY.PEEK[]`, flags, MOVE with COPY fallback. Mailbox names are quoted; CR/LF are refused. |
| L2 | `smtp` | Sending through `lettre` over rustls (implicit TLS or STARTTLS, platform verifier), plain-text messages with Date, In-Reply-To and References; the Message-ID is minted under the sender's domain so the computer's host name never goes on the wire. |
| L2 | `mailbox` | The `Mailbox` trait every mail store implements (folders, headers, bodies, flags, moves, send), with opaque string ids so the engine never knows whether it talks to IMAP or EWS; and `ItemStore`, which a store that also keeps appointments, contacts and tasks offers through `Mailbox::items`. |
| L2 | `exchange` | Exchange Web Services over HTTPS (`ureq` + rustls, platform verifier): `FindFolder`/`GetFolder`, `FindItem`, `GetItem` with MIME content (parsed by `mime`, like IMAP mail), `UpdateItem`, `MoveItem`, `CreateItem` to send. Calendar, contacts and tasks as an `ItemStore`: `FindItem` (a `CalendarView` for appointments) for ids, `GetItem` in batches for fields and body, `CreateItem`/`UpdateItem`/`DeleteItem` to write back; the request header names the computer's time zone (IANA name mapped to the Windows name) so all-day appointments and due days land on the right day. Every request value is XML-escaped; replies are size-capped and read with a read-only parser. Basic or Bearer authentication only; NTLM is reported, not spoken. |
| L2 | `oauth` | OAuth 2.0 authorization-code flow with PKCE (RFC 8252): the system browser signs in, a loopback listener on `127.0.0.1` with a random port receives the code once, `state` is checked, tokens are fetched over rustls. No embedded web view; only the refresh token is stored (keychain). |
| L3 | `engine` | `Session` (state and selection) and the command registry: every action is a `CommandSpec` with an id, run through `Session::run`, which validates parameters and never panics on bad input. One worker thread per account speaks IMAP/SMTP or EWS with a password or an OAuth access token refreshed on demand, and mirrors the account's appointments, contacts and tasks when its store keeps them (`sync`). Built-in sample mail. |
| L4 | `ui-egui` | Outlook-style front end drawn with egui: tab strip and ribbon, module rail (Mail, Calendar, People, Tasks), folder pane, message list, reading pane, status bar; calendar with month, week and day views and an appointment window (`chrono` for dates and the local time zone). Light and dark theme tokens in the ArtCraft style. |
| apps | `mailcraft` | Desktop app (eframe + wgpu). |

## Calendar

Appointments are session state like messages: `Session::appointments` plus `appointment.select`, `appointment.save` and `appointment.delete` commands, each validated in `model`. The engine saves `calendar.json` (owner-only, written through a temporary file) right after a command changes it. Times are Unix seconds in UTC; only the UI converts to the computer's time zone, so a file moved between machines keeps its moments. An account whose store keeps a calendar (Exchange) gets a calendar of its own in the session, see Account items below; the local calendar stays as the fallback. Planned: Google Calendar and CalDAV behind the same `ItemStore` trait.

## Contacts

Contacts follow the calendar's pattern: `Session::contacts` with `contact.select`, `contact.save` and `contact.delete`, validated in `model` (every address must parse), saved to `contacts.json` right after a command changes them. `Session::suggest_addresses` serves the compose window: contacts first, then addresses seen in the session's mail, matched on the start of a word or of the address. A contact carries the account it lives in (`None` for this computer); the form asks where a new one goes when there is a choice. Planned: Google People and CardDAV behind the same `ItemStore` trait.

## Tasks

Tasks follow the same pattern: `Session::tasks` with `task.select`, `task.save`, `task.complete` and `task.delete`, validated in `model`, saved to `tasks.json`. A due day is stored as local midnight in Unix seconds, so "overdue" is a plain comparison with today's midnight. Open tasks sort by due day, then priority; done ones by completion. Tasks carry their account like contacts do. Planned: Google Tasks behind the same `ItemStore` trait.

## Account items

`engine::sync` mirrors the appointments, contacts and tasks of an account whose `Mailbox` offers an `ItemStore`. When such an account connects (`Event::Connected { syncs_items: true }`) the engine queues `Job::Sync` for a window of 90 days back and 400 days ahead; `sync.refresh` (the Refresh button in the calendar, contacts and tasks ribbons) asks again. `Event::Synced` replaces the account's items in the session. Session ids of account items are `<account id>/<store id>`, like folder ids, so an id always says where an item lives, and the local files (`calendar.json`, `contacts.json`, `tasks.json`) receive local items only. A command on an account's item changes the session at once and leaves `Job::SaveItem` or `Job::DeleteItem` for the store; a new item carries a pending id until `Event::ItemSaved` brings the store's id, and editing it before then is refused rather than duplicated. All-day appointments and due days come back from a store as midnight in its time zone; the engine snaps them to the nearest local midnight. There is no offline cache of account items yet: without a connection the modules show the local items only (the planned `store` crate covers that).

## Planned crates (mail module)

| Crate | Job | Security notes |
|---|---|---|
| `html` | Lay out sanitised HTML mail in the reading pane (today HTML is shown as text) | Allow-list of tags and attributes; no scripts, forms, external loads; CSS restricted; remote content blocked by default |
| `store` | Local encrypted database (SQLCipher-style) with full-text search and offline queue | Key in the system keychain; search index inside the same encrypted file |
| `sync` | Background sync, offline queue, conflict handling | Per-account rate limits and back-off |
| `crypto` | S/MIME and OpenPGP signing, encryption and verification | Pure-Rust implementations |

## OWASP top 10 mapping

| Risk | How Mailcraft handles it |
|---|---|
| Injection | Addresses and headers validated in `model`; SQL through parameterised queries only; IMAP/SMTP commands built from typed values, never string concatenation |
| Broken authentication | OAuth2 + PKCE where offered; passwords only over TLS; keychain storage |
| Sensitive data exposure | Encrypted local store; no telemetry; remote content blocked; logs never contain addresses or bodies |
| XML/XSS-class issues | No browser engine; HTML sanitised to an allow-list; no script execution anywhere |
| Security misconfiguration | Secure defaults only: no plain-text option, certificate checks cannot be disabled in the UI |
| Vulnerable components | `cargo audit` and `cargo deny` in CI; pure-Rust dependencies preferred |
| Integrity | Signed releases; S/MIME and PGP verification shown clearly in the reading pane |
| Logging | Command ids and errors are logged; content is not |
| SSRF-class issues | Only user-configured hosts are contacted; no automatic fetches from mail content |
