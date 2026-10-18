# Mailcraft architecture

| Layer | Crate | Job |
|---|---|---|
| L1 | `model` | Accounts, folders, message headers, addresses. Validates everything that comes from the network (address shape, header length, control characters). No I/O. |
| L2 | `engine` | `Session` (state and selection) and the command registry: every action is a `CommandSpec` with an id, run through `Session::run`, which validates parameters and never panics on bad input. Built-in sample mail. |
| L3 | `ui-egui` | Outlook-style front end drawn with egui: tab strip and ribbon, module rail (Mail, Calendar, People, Tasks), folder pane, message list, reading pane, status bar. Light and dark theme tokens in the ArtCraft style. |
| apps | `mailcraft` | Desktop app (eframe + wgpu). |

## Planned crates (mail module)

| Crate | Job | Security notes |
|---|---|---|
| `mime` | Parse RFC 5322 / MIME, decode bodies and attachments | Bounded sizes and depth; fuzzed; never panics |
| `html` | Sanitise HTML mail to a safe subset and lay it out for the reading pane | Allow-list of tags and attributes; no scripts, forms, external loads; CSS restricted; remote content blocked by default |
| `imap`, `smtp` | Protocol clients over rustls | TLS 1.2+ only; STARTTLS is required, never optional; certificate validation on; OAuth2 (XOAUTH2) and SASL PLAIN |
| `store` | Local encrypted database (SQLCipher-style) with full-text search | Key in the system keychain; search index inside the same encrypted file |
| `secrets` | System keychain (macOS Keychain, Windows Credential Manager, Secret Service) | Zeroise in memory; no secrets in logs or settings |
| `sync` | Background sync, offline queue, conflict handling | Per-account rate limits and back-off |
| `crypto` | S/MIME and OpenPGP signing, encryption and verification | Pure-Rust implementations |
| `accounts` | Account set-up: IMAP/SMTP autoconfig, then Gmail, Microsoft 365 and Exchange (EWS/Graph) | OAuth2 with PKCE through the system browser; no embedded web view |

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
