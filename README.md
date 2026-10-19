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
delete, archive, move to junk or to any folder of the account ("Verplaatsen" in the ribbon, or drag
the message onto a folder); changes go back to the server. A folder shows its newest 200
messages and a button at the bottom of the list fetches the next 200, as often as needed. New mail
comes in on its own: the inbox of every connected account and the open folder are refreshed every
two minutes (calendar, contacts and tasks every five), the status bar counts new unread messages,
the window asks for attention, and its title shows the unread mail in the inboxes of every account
together ("Mailcraft (3)"), so the count is visible in the dock, taskbar or window switcher. Junk: the
server's own filter stays the first line, and its Junk folder shows as "Ongewenste e-mail". On top of that,
"Afzender blokkeren" in a message's right-click menu (or "Bestand" > "Ongewenste e-mail", for an address or a
whole domain) keeps a list of blocked senders in `junk.json`; new mail from one of them goes straight to that
folder, like "Ongewenst" by hand. Safe senders and your contacts are never filed, and "Geen ongewenst" (on a
message in the Junk folder, or in its menu) brings a message back and trusts its sender. The lists live on
your computer; nothing about your mail is sent anywhere to be checked. Phishing: a message that shows signs of it
gets a warning strip above its text that names them, with "Afzender blokkeren" at hand: a sender who uses a
contact's name (or an address as a name) with another address, a domain one letter or one look-alike letter away
from a domain you know (your contacts', your own), failed SPF/DKIM/DMARC checks reported by your mail server in the
`Authentication-Results` headers, a link whose text shows one site while it goes to another, and an attachment that
runs as a program. The checks run on your computer only and never decide for you. A search box above the
message list narrows it by subject, sender, recipient or preview text, in the current folder or
across every folder of every account (it searches what Mailcraft has fetched so far); "Zoek op de server" asks the mail server itself, which also looks in older mail and in the text of the messages. Sending over SMTP: new mail,
reply, reply all and forward, with attachments (picked with the file dialog or dropped onto the window, 25 MB together), a copy filed in Sent and the account's signature under the text. Closing a message you were writing puts it aside as a draft ("Concepten (deze computer)" under the account, kept in the encrypted offline cache, never on the server) to open, finish and send later. A password account's name,
servers, user name and password can be changed afterwards from the account list. Gmail and Microsoft 365 sign in through
the system browser (OAuth 2.0 with PKCE; XOAUTH2 on IMAP and SMTP for Gmail, Microsoft Graph
for Microsoft 365); see [docs/accounts.md](docs/accounts.md) for the one-time app
registration. Exchange Server (2013+)
works through Exchange Web Services over HTTPS, reading and sending alike. The calendar module
shows a month, week or day view with a local calendar: appointments are created, edited and
deleted in a window and kept in `calendar.json`. An appointment in a calendar on this computer can
repeat (every so many days, weeks on chosen weekdays, months or years; for ever, until a date or a
number of times): the file keeps the rule once, the views show every occurrence, and one
occurrence can be skipped from the window without touching the rest. A new appointment in an
account's calendar can repeat as well: the rule goes to the server (Exchange and Microsoft 365 as
their recurrence pattern, Google and CalDAV as an iCalendar `RRULE`, each in this computer's time
zone), which then owns the series and lists its occurrences, edited one by one as the server has
them. Every appointment can carry a
reminder (a quarter of an hour before by default, from "op het moment zelf" to a week ahead); when
it is due a window lists it with the time left, to open the appointment, close the reminder or put
it off for a while, and the app asks for attention once. Reminders travel with Exchange, Microsoft
365, Google and CalDAV appointments as the server's own reminder or alarm. The contacts module keeps a local address
book (`contacts.json`) with search and a contact card, and the compose window completes
addresses from it and from the mail you have seen. A sender goes into the address book in one click: "Toevoegen aan
contacten" next to the sender in the reading pane (and "Afzender toevoegen aan contacten" in the message's right-click
menu) opens the contact form with the name and address filled in; when the address is already known the button reads
"Contact openen" and shows that card. The tasks module keeps a local to-do list
(`tasks.json`) with due days, priorities, an open/done filter and a search box. The calendar has a search box
too: while it holds words, a list of the matching appointments of the past year and the two years ahead
(title, location or notes; one row per occurrence of a series) replaces the view, and a click opens the
appointment. An appointment can be dragged to another day in the month view, or to another day and half
hour in the week and day views; it keeps its length and everything else, in an account's calendar the move
goes to the server, and a repeating series is changed through its window instead. In the week and day views
the bottom edge of an appointment can be dragged too, to make it longer or shorter by the half hour. The calendar's
side pane lists the open tasks; drag one onto a day or an hour and the appointment window opens for it, with the
task's title and notes filled in, so planning a task is one drag and "Opslaan". In all three modules Ctrl+E goes to the search box and Esc clears it, as in the mail module. An Exchange, Microsoft 365 or Gmail account
brings its own calendar, contacts and tasks along (Google Calendar, People and Tasks for Gmail), and so
does an IMAP account with a CalDAV/CardDAV server next to it (Fastmail, iCloud, Nextcloud and the like): they
appear next to the local ones, changes go straight to the server, and a new item asks whether
it belongs on this computer or in the account. Everything an account showed is kept in an
encrypted offline cache, so the window fills at once on the next start and mail you opened
before can be read without a connection, and changes you make without one (flags, moves,
appointments, contacts, tasks) wait and reach the server when it is back, even after a restart.

Folders are yours to arrange: the "Map" tab in the ribbon (or a right click on a folder) makes
a new folder under the selected one, renames an ordinary folder or removes it with everything in
it, after asking; the inbox and the other special folders stay as they are. The change goes to
the server at once (IMAP `CREATE`/`RENAME`/`DELETE`, Exchange Web Services, Microsoft Graph) and
the folder pane shows the list the server answers with, so it needs a connection. The same tab
and menu empty a folder ("Map leegmaken", after asking: everything in it goes for good, on the
server too) and mark all its mail read. Deleting a message that is already in "Verwijderde
items" removes it for good, after a question; Exchange and Microsoft 365 keep such mail
recoverable on the server for a while, as they do for Outlook.

Rules sort incoming mail: "Regels" in the ribbon opens a window where a rule says which part of a
message (sender, recipient, subject or the whole envelope) must contain which text, and what then
happens (move to a folder, mark as read, flag, give one or more categories). Rules run over the mail a refresh finds new in an
account's inbox, also while offline (the change waits in the queue like any other), and "Regels nu
uitvoeren op deze map" runs them over the open folder at once; "Regel voor deze afzender" starts one
from the selected message. Rules are kept in `rules.json`.

Quick steps are your own buttons on the "Start" tab, as in Outlook: "Beheren…" in the "Snelle
stappen" group opens a window where a step gets a name and what it does to the selected message
in one go (move to a folder, mark as read, flag, give categories). Each step is a button in the
ribbon, and the first nine of an account also answer to Ctrl+Shift+1 to 9. They are kept in
`quick_steps.json`.

Categories, the coloured labels Outlook users know, go on a message through "Categoriseren" in
the ribbon: Outlook's six to begin with, and "Alle categorieën…" adds, recolours or removes
them (`categories.json`). The list shows a coloured square per category, the open message its
chips (a click takes one off). The names travel with the mail: Exchange and Microsoft 365 keep
them as the item's categories, IMAP as keywords (spaces become `_`), so other clients see them
too; a category another client set shows in grey until you give it a colour here.

The "Beeld" tab chooses what the list shows and how: only unread, flagged or mail with
attachments, or only mail with a chosen category; sorted newest or oldest first, by sender or by subject; the reading pane to the
right or below the list; compact rows without the preview line; "Gesprekken", which folds the messages on one
subject (whatever their "Re:" or "Fwd:" prefixes) behind the one that stands first in the list, with an arrow to
unfold them; and a light, dark or system theme. The choices are kept in `view.json` next to the accounts file.

"Afdrukken" under "Bestand" (or Ctrl+P) prints the open message by way of the system browser: Mailcraft writes a
page of its own (sender, recipients, date, subject, attachment names and the body as it drew it,
with embedded pictures and without anything from the internet or any script) to one file in the
temporary folder and opens it, and the browser's print dialog does the rest.

Keyboard shortcuts in the mail module follow Outlook (Cmd instead of Ctrl on a Mac): arrow keys
walk the message list, Delete removes the selected message, Backspace archives it (on a Mac, where Backspace is
the delete key, Ctrl+E archives, as in Outlook for Mac), Ctrl+N starts a new message, Ctrl+R
replies, Ctrl+Shift+R replies to all, Ctrl+F forwards, Ctrl+Shift+V moves to a folder, Ctrl+Q and Ctrl+U mark read or unread,
Ctrl+E puts the cursor in the search box, Ctrl+P prints, Ctrl+Shift+1 to 9 run the account's quick steps, Esc clears a
search and F5 refreshes the folder. From every module, Ctrl+Shift+M, Ctrl+Shift+A, Ctrl+Shift+C and Ctrl+Shift+K start a
new message, appointment, contact or task, as in Outlook (the contact and task forms open in their own module). None
of them fires while you are typing in a text field or writing a message. A right click on a message opens a menu with
the everyday actions (reply, forward, read or unread, flag, move, archive, junk, delete) and two Outlook favourites:
"Taak maken van dit bericht" and "Afspraak maken van dit bericht", which open a new task or appointment named after the
message, with the sender, the date and the text as notes. Hovering a ribbon button
shows its shortcut, and the "Help" tab lists them all in one window; the same tab opens the folder
that holds your settings and cache, the project page, and "Over Mailcraft" with the version.

## Build

```
cargo run -p mailcraft
cargo test
cargo clippy --all-targets -- -D warnings
```

Requires Rust 1.95 or newer.

See [`docs/architecture.md`](docs/architecture.md).
