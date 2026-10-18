# Mailcraft

Mail and calendar for the desktop, with the Outlook workflow you already know and the look of the
ArtCraft family (WordCraft, GridCraft, DeskCraft). Pure Rust, one binary, runs on macOS, Windows
and Linux.

## Principles

- **Security and privacy by design.** No browser engine in the app, so mail cannot run scripts.
  HTML mail is sanitised before it is shown; remote images are blocked until you allow them.
  Transport is TLS only. Credentials live in the system keychain, never in files. `unsafe` is
  forbidden across the workspace and nothing user-facing may panic.
- **One command list.** Every action (open a folder, delete, flag, archive) is a command id run
  through `Session::run`, the same way the ribbon, keyboard and later scripting reach it.
- **Layered.** Model → engine → UI. Nothing below the UI depends on egui.

## Status

The Outlook-style window (ribbon, module rail, folder pane, message list, reading pane) over
built-in sample mail, and reading mail from an IMAP account: add it under File › Accounts,
browse folders, open messages (plain text, or HTML reduced to text), mark read or unread, flag,
delete, archive and move to junk; changes go back to the server. Sending over SMTP: new mail,
reply, reply all and forward, with a copy filed in Sent. Gmail and Microsoft 365 sign in through
the system browser (OAuth 2.0 with PKCE, XOAUTH2 on IMAP and SMTP); see
[docs/accounts.md](docs/accounts.md) for the one-time app registration. Next up: Exchange;
after mail come calendar, contacts and tasks.

## Build

```
cargo run -p mailcraft
cargo test
cargo clippy --all-targets -- -D warnings
```

Requires Rust 1.95 or newer.

See [`docs/architecture.md`](docs/architecture.md).
