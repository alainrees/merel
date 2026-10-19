# Mailcraft

Mail and calendar for the desktop, with the Outlook workflow you already know and the look of the
ArtCraft family (WordCraft, GridCraft, DeskCraft). Pure Rust, one binary, runs on macOS, Windows
and Linux.

## Principles

- **Security and privacy by design.** No browser engine in the app, so mail cannot run scripts.
  HTML mail is reduced to a layout tree before it is drawn; pictures on the internet are blocked.
  Transport is TLS only. Credentials live in the system keychain, never in files. `unsafe` is
  forbidden across the workspace and nothing user-facing may panic.
- **One command list.** Every action (open a folder, delete, flag, archive) is a command id run
  through `Session::run`, the same way the ribbon, keyboard and later scripting reach it.
- **Layered.** Model → engine → UI. Nothing below the UI depends on egui.

## Status

The Outlook-style window (ribbon, module rail, folder pane, message list, reading pane) over
built-in sample mail, and reading mail from an IMAP account: add it under File › Accounts,
browse folders, open messages (plain text, or HTML drawn with its headings, lists, tables, links
and embedded pictures, while pictures on the internet stay blocked until you click "Afbeeldingen
laden" for that one message; a click on an attachment saves it to your Downloads folder), mark read or unread, flag,
delete, archive and move to junk; changes go back to the server. A folder shows its newest 200
messages and a button at the bottom of the list fetches the next 200, as often as needed. New mail
comes in on its own: the inbox of every connected account and the open folder are refreshed every
two minutes (calendar, contacts and tasks every five), the status bar counts new unread messages
and the window asks for attention. A search box above the
message list narrows it by subject, sender, recipient or preview text, in the current folder or
across every folder of every account (it searches what Mailcraft has fetched so far). Sending over SMTP: new mail,
reply, reply all and forward, with attachments (picked with the file dialog or dropped onto the window, 25 MB together), a copy filed in Sent and the account's signature under the text. A password account's name,
servers, user name and password can be changed afterwards from the account list. Gmail and Microsoft 365 sign in through
the system browser (OAuth 2.0 with PKCE; XOAUTH2 on IMAP and SMTP for Gmail, Microsoft Graph
for Microsoft 365); see [docs/accounts.md](docs/accounts.md) for the one-time app
registration. Exchange Server (2013+)
works through Exchange Web Services over HTTPS, reading and sending alike. The calendar module
shows a month, week or day view with a local calendar: appointments are created, edited and
deleted in a window and kept in `calendar.json`. The contacts module keeps a local address
book (`contacts.json`) with search and a contact card, and the compose window completes
addresses from it and from the mail you have seen. The tasks module keeps a local to-do list
(`tasks.json`) with due days, priorities and an open/done filter. An Exchange, Microsoft 365 or Gmail account
brings its own calendar, contacts and tasks along (Google Calendar, People and Tasks for Gmail), and so
does an IMAP account with a CalDAV/CardDAV server next to it (Fastmail, iCloud, Nextcloud and the like): they
appear next to the local ones, changes go straight to the server, and a new item asks whether
it belongs on this computer or in the account. Everything an account showed is kept in an
encrypted offline cache, so the window fills at once on the next start and mail you opened
before can be read without a connection, and changes you make without one (flags, moves,
appointments, contacts, tasks) wait and reach the server when it is back, even after a restart.

## Build

```
cargo run -p mailcraft
cargo test
cargo clippy --all-targets -- -D warnings
```

Requires Rust 1.95 or newer.

See [`docs/architecture.md`](docs/architecture.md).
