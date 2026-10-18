# Mailcraft architecture

| Layer | Crate | Job |
|---|---|---|
| L1 | `secrets` | Passwords in the system credential store (Keychain, Credential Manager, Secret Service) through `keyring`; a memory store for tests. Secrets are zeroed on drop and hidden from `Debug`. |
| L1 | `model` | Accounts, folders, message headers, addresses, calendars and appointments. Validates everything that comes from the network or the keyboard (address shape, header length, control characters, appointment times). No I/O. |
| L2 | `mime` | Header and body parsing through `mail-parser`; HTML is reduced to plain text with `html2text`, so no script, style or remote reference survives. Input is bounded at 8 MB. |
| L2 | `imap` | Blocking IMAP client over rustls (implicit TLS or STARTTLS, platform certificate verifier, no way to disable checks): folders with SPECIAL-USE, newest headers with BODYSTRUCTURE, `BODY.PEEK[]`, flags, MOVE with COPY fallback. Mailbox names are quoted; CR/LF are refused. |
| L2 | `smtp` | Sending through `lettre` over rustls (implicit TLS or STARTTLS, platform verifier), plain-text messages with Date, In-Reply-To and References; the Message-ID is minted under the sender's domain so the computer's host name never goes on the wire. |
| L2 | `mailbox` | The `Mailbox` trait every mail store implements (folders, headers, bodies, flags, moves, send), with opaque string ids so the engine never knows whether it talks to IMAP or EWS. |
| L2 | `exchange` | Exchange Web Services over HTTPS (`ureq` + rustls, platform verifier): `FindFolder`/`GetFolder`, `FindItem`, `GetItem` with MIME content (parsed by `mime`, like IMAP mail), `UpdateItem`, `MoveItem`, `CreateItem` to send. Every request value is XML-escaped; replies are size-capped and read with a read-only parser. Basic or Bearer authentication only; NTLM is reported, not spoken. |
| L2 | `oauth` | OAuth 2.0 authorization-code flow with PKCE (RFC 8252): the system browser signs in, a loopback listener on `127.0.0.1` with a random port receives the code once, `state` is checked, tokens are fetched over rustls. No embedded web view; only the refresh token is stored (keychain). |
| L3 | `engine` | `Session` (state and selection) and the command registry: every action is a `CommandSpec` with an id, run through `Session::run`, which validates parameters and never panics on bad input. One worker thread per account speaks IMAP/SMTP with a password or an XOAUTH2 access token refreshed on demand. Built-in sample mail. |
| L4 | `ui-egui` | Outlook-style front end drawn with egui: tab strip and ribbon, module rail (Mail, Calendar, People, Tasks), folder pane, message list, reading pane, status bar; calendar with month, week and day views and an appointment window (`chrono` for dates and the local time zone). Light and dark theme tokens in the ArtCraft style. |
| apps | `mailcraft` | Desktop app (eframe + wgpu). |

## Calendar

Appointments are session state like messages: `Session::appointments` plus `appointment.select`, `appointment.save` and `appointment.delete` commands, each validated in `model`. The engine saves `calendar.json` (owner-only, written through a temporary file) right after a command changes it. Times are Unix seconds in UTC; only the UI converts to the computer's time zone, so a file moved between machines keeps its moments. Planned: calendars per account (EWS `CalendarView`, Google Calendar, CalDAV) behind a `CalendarStore` trait like `Mailbox`, with the local calendar as the fallback.

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
