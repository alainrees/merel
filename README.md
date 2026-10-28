# Merel

Mail and calendar for the desktop, with the Outlook workflow you already know. Pure Rust, one
binary, runs on macOS, Windows and Linux. Merel is Dutch for blackbird; the icon is a folded paper
one. The app was called Mailcraft until October 2026; settings, passwords and appointments from
that time carry over on the first start.

The windows are moving to the "Helder" style, a fresher look after the new Outlook: white rounded
cards on a light grey ground, quiet buttons that are a line icon and a label (Lucide icons, ISC
licence, embedded), one blue main button per bar, fields with only an underline and switches
instead of tick boxes. The appointment window is the first: a command bar on top, the form with an
icon per row, and the day it falls on beside it, so a clash shows before you save.

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
laden" for that one message, or "Altijd van deze afzender", which puts the sender on the safe senders list so their pictures load from then on without asking, as Outlook does for Safe Senders; a click on an attachment saves it to your Downloads folder ("Alle bijlagen opslaan" after the
attachments saves every one of a message with several, each under its own name, as in Outlook), and a picture attachment
also shows a thumbnail that opens a preview window, as in Outlook; a meeting invitation (a `text/calendar` part)
gets a strip with the appointment, its time, place and organiser and "Aan agenda toevoegen", which opens the
appointment window filled in for your own calendar; nothing is sent back to the organiser; a message sent with high or
low importance shows a red "!" or a blue arrow before its date and a strip over the text, and "Urgentie" in the compose
window sends your own that way, as the `Importance` and `X-Priority` headers), mark read or unread, flag,
delete, archive, move to junk or to any folder of the account ("Verplaatsen" in the ribbon, or drag
the message onto a folder); changes go back to the server. A folder shows its newest 200
messages and a button at the bottom of the list fetches the next 200, as often as needed. New mail
comes in on its own: the inbox of every connected account and the open folder are refreshed every
two minutes (calendar, contacts and tasks every five), a notice counts new unread messages,
the window asks for attention, and its title shows the unread mail in the inboxes of every account
together ("Merel (3)"), so the count is visible in the dock, taskbar or window switcher. As in Outlook,
that count also stands in red on the app icon (the Dock on macOS, the launcher on Linux desktops that show
one), and while Merel is in the background the computer's own notification announces new mail and due
reminders; a click on one opens the message, appointment or task. "Bestand" > "Meldingen" switches each of
these on or off; the notification names no sender, subject or title unless that is switched on there too,
so nothing is read off a locked screen. On macOS the red window button closes the window but not Merel, as in
Outlook for Mac: mail, notifications and the count go on, a click on the Dock icon brings the window back, and
"Merel" > "Stop Merel" (Cmd+Q) ends it. Junk: the
server's own filter stays the first line, and its Junk folder shows as "Ongewenste e-mail". On top of that,
"Afzender blokkeren" in a message's right-click menu (or "Bestand" > "Ongewenste e-mail", for an address or a
whole domain) keeps a list of blocked senders in `junk.json`; new mail from one of them goes straight to that
folder, like "Ongewenst" by hand. Safe senders and your contacts are never filed, and "Geen ongewenst" (on a
message in the Junk folder, or in its menu) brings a message back and trusts its sender. "Ongewenst", "Afzender
blokkeren" and "Geen ongewenst" also tell the server what you decided, so its filter learns and your phone sees the
same: the `$Junk`/`$NotJunk` keywords over IMAP, `MarkAsJunk` on Exchange (which also keeps the sender in the
mailbox's blocked list); Gmail learns from the move to Spam itself and Microsoft 365 over Graph from the move to
Junk Email. The window also sets the level, as Outlook's junk options do: "Standaard" files only the blocked
senders, "Hoog" also a sender whose header shows signs of phishing (a contact's name with another address, a
look-alike of a known domain), and "Alleen veilige lijsten" everything that is not from a contact, a safe sender or
one of your own addresses. Besides the lists, a learning filter ("Lerend filter" in the same window, on unless
turned off) learns from what you file as junk or take back out of the Junk folder, and from mail of your contacts
and safe senders, and once it knows ten of each files new mail that looks like your junk (Robinson's chi-squared
method, as SpamBayes does it). What it learned stays in `junk-learned.json` as salted hashes, without a readable
word, and "Vergeten wat het filter leerde" empties it. The lists live on your computer; nothing about your mail is
sent anywhere to be checked.
When something gets past all that, "Melden" over the message (or "Melden als phishing of spam" in its right-click
menu) sends its source to the Merel team so the check can learn from it: only on that click, from your own account,
as an attached `.eml` with your names and addresses replaced by "Ontvanger" (in encoded text parts too), after
which the message moves to Junk.
"Gesprek negeren" (in a message's right-click menu, or Ctrl+Delete) is Outlook's Ignore: the messages of
that conversation (same subject, prefixes aside) in the inbox go to "Verwijderde items", and so does every message
that arrives in it afterwards; a strip above such a message has "Niet meer negeren", which brings them back. The
list is `ignored.json`, per account. "Gesprek opruimen…" (same menu) and "Map opruimen…" (a folder's menu, or
"Opruimen" on the Map tab) are Outlook's Clean Up: of each conversation the older messages go to "Verwijderde
items" and the newest stays, as do unread, flagged and categorised messages and messages with attachments; Merel
asks first and says how many would go. It goes by the headers it has, not by whether a later reply quotes the whole
earlier message. Phishing: a message that shows signs of it
gets a warning strip above its text that names them, with "Afzender blokkeren" at hand: a sender who uses a
contact's name (or an address as a name) with another address, a domain one letter or one look-alike letter away
from a domain you know (your contacts', your own), failed SPF/DKIM/DMARC checks reported by your mail server in the
`Authentication-Results` headers, a link whose text shows one site while it goes to another, and an attachment that
runs as a program. The checks run on your computer only and never decide for you. In the Junk folder itself a
message is shown with its links, attachments and pictures from the internet switched off, as Outlook does; "Geen
ongewenst" brings it back with everything working. A search box above the
message list narrows it by subject, sender, recipient or preview text, across every folder of every account
(the start) or in the current folder, as last chosen in the menu next to the box. Like Outlook and Apple Mail, Merel keeps a search index of all mail on this
computer (`merel-search`, built on Tantivy): it pages quietly through every folder in the background, one small job per
account at a time, and fetches the text of each message, newest first, as far back as "Bestand > Zoeken" says (all mail,
or the last 5, 2 or 1 years; older mail is found by sender, recipients and subject). A search then covers old mail too,
works without a connection and answers at once: accents do not matter (`cafe` finds `café`), Dutch and English word
forms count (`factuur` finds `facturen`), a typing error or two is forgiven (`fatcuur`), the word being typed counts as
the start of a word, hits stand best match first (subject and sender weigh most, recent mail and known people rise; the
scope menu switches to newest first) and each row shows the piece of text where the words were found, marked. The
words inside attachments count too: the text of PDF, Word, Excel, PowerPoint and OpenDocument files, HTML, forwarded
messages and plain text (csv, an e-invoice in XML) is read in the background, on this computer, with limits on what a
file may unpack to, and a hit there says "In bijlage offerte.pdf:" before its piece of text. The index
is sealed on disk under the offline cache's key (one sealed file per index file, decrypted in memory only), the text of
encrypted mail stays out of it, a day later each folder is paged through again so mail that left the server leaves the
index, and "Bestand > Zoeken" shows how far it is, switches it off (which deletes it) or clears it. Outlook's search words work in it:
`van:sanne`, `aan:jan` (also `cc:`) and `onderwerp:offerte` look in one field, `heeft:bijlage`, `is:ongelezen` and
`is:gemarkeerd` keep only such messages, `bijlage:pdf` looks at the names of attachments, `categorie:rood` at the
categories, `map:facturen` and `account:werk` search those folders whatever the scope, and `datum:` (`vandaag`,
`gisteren`, `deze-week`, `vorige-week`, `deze-maand`, `vorige-maand`, `dit-jaar`, `vorig-jaar`, `2026`, `2026-10`,
`2026-10-24` or `24-10-2026`), `voor:` and `na:` narrow by when. Merel also recognises, by fixed rules and on this
computer, amounts of money, IBANs (checked by their check digits), track-and-trace codes and order and invoice numbers in
the text and the attachments: `bedrag:>100`, `bedrag:<50`, `bedrag:100-500` or `bedrag:1250` find mail by an amount, and
`heeft:bedrag`, `heeft:iban`, `heeft:zending`, `heeft:ordernummer` and `heeft:factuurnummer` keep mail that holds one.
`groot:>5MB` (also `grootte:` or `size:`, with `<`, `>=`, `<=` or a range such as `1MB-10MB`, in KB, MB or GB) finds mail
by its size, as the server counts it. In the message being read, the words a search found stand on a yellow ground, in
whatever form they were found (another ending, without accents, or near the typo in the search box).
A word is also searched in Dutch, English, German and French: `voorstel` finds mail about an `offerte`, a `quote`, an
`Angebot` or a `devis`, from a built-in list of a few hundred words of mail and business (`crates/search/src/lexicon.txt`).
The word as typed ranks first; "Ook gezocht: …" above the results names the others and switches them off, and a word in
quotes is searched only as it stands.
An opened message shows what was recognised in a "Herkend:" strip: a click on an order or invoice number searches all mail
for it, a parcel code with a known carrier (PostNL, DHL, DPD, UPS, GLS) opens that carrier's page, an amount or IBAN is
copied (not in the Junk folder). Search on meaning, which "Bestand > Zoeken" switches on (it is off until then), also
finds mail about the same subject without the words: a search for `verbouwing` brings up the builder's mail about the
kitchen. Switching it on downloads an open language model once (`intfloat/multilingual-e5-small` from Hugging Face, at a
fixed revision, checked by SHA-256; trimmed to the Latin-script vocabulary and stored at half precision), and from then
on everything runs on this computer: in the background, at an easy pace, Merel turns each message it has the text of into
a vector, sealed on disk like the index, and a search mixes the closest messages in under the ones with the words
(reciprocal rank fusion). Such a row says "Over hetzelfde". Switching it off throws the model and the vectors away. A phrase in quotes must occur as it stands, `-woord`
leaves out what holds it, and `OF` between words lets either do (the English `from:`, `to:`, `subject:`, `has:attachment`,
`is:unread`, `is:flagged`, `date:`, `before:`, `after:`, `folder:`, `attachment:`, `category:`, `OR`, `today`, `this-week`
and so on too); the rest are plain words that may occur anywhere. While a word is typed, a list under the box suggests
people (contacts first, then senders by how much mail they sent), search words, folders, categories, date words and earlier
searches; a click or Tab puts one in. Under a search the chips Datum, Van and Map (the senders and folders of what was
found) put those search words in the box or take them out, and "Met bijlage" and "Alleen ongelezen" switch those on or off. "Zoek op de server" asks the mail server itself, which also looks in older mail and in the text of the messages; the server gets the plain words, and what it finds is then held to the same search words here. "Zoekmap" next to the box saves the search under a name, as an Outlook search folder: it is listed under "Zoekmappen" in the folder pane with its words, scope and list filter, a click runs it again, and its menu removes it (`view.json`). The 🕒 menu next to the box lists the last ten searches, as Outlook's "Recente zoekopdrachten": a click searches those words again, "Lijst wissen" forgets them (`view.json`). Sending over SMTP: new mail,
reply, reply all and forward (a forwarded message takes its attachments along, as in Outlook, except from the Junk folder; "Doorsturen als bijlage" in the message menu, or Ctrl+Alt+F, sends the message itself along as an .eml file with its headers, like Outlook's "Forward as Attachment", not from the Junk folder either; "Opnieuw verzenden" opens a message, a sent one say, as a new message with the same recipients, subject, text and attachments, like Outlook's "Resend This Message"), with attachments (picked with the file dialog or dropped onto the window, 25 MB together), a copy filed in Sent and the account's signature under the text. Closing a message you were writing puts it aside as a draft ("Concepten (deze computer)" under the account, kept in the encrypted offline cache, never on the server) to open, finish and send later. "Later verzenden" in the same window puts the message aside with a moment (in an hour, this evening, tomorrow morning, Monday morning), like Outlook's "Bezorging uitstellen": it shows under Concepten with that moment, Merel sends it then if it is running, and opening it lets you drop the moment or send at once; one that cannot go becomes an ordinary draft again with a notice. Before a message goes, Merel asks once, as Outlook does, when the text speaks of an attachment ("bijlage", "bijgevoegd", "attached", …) while none is on, or when the subject is empty; "Toch verzenden" sends it as it is. A password account's name,
servers, user name and password can be changed afterwards from the account list. Gmail and Microsoft 365 sign in through
the system browser (OAuth 2.0 with PKCE; XOAUTH2 on IMAP and SMTP for Gmail, Microsoft Graph
for Microsoft 365). Microsoft 365 uses Merel's own Entra registration, so only name and address are asked,
with room for a client ID of your own; Gmail needs a client of your own, see [docs/accounts.md](docs/accounts.md). Exchange Server (2013+)
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
it off for a while, and the app asks for attention once. The same window names a flagged message from its follow-up
day on (as Outlook reminds of flagged items), to open it, mark it done, close the reminder or put it off; closing is
kept in `follow_ups.json`, a new follow-up day opens it again. A task with a due day reminds in the same window from eight
in the morning of that day until it is done, as Outlook does for tasks with a due date, with "Openen" (the task in its
module), "Voltooid", "Sluiten" and "Uitstellen"; closing is kept in `tasks.json` for that due day, so a new due day
reminds again. Reminders travel with Exchange, Microsoft
365, Google and CalDAV appointments as the server's own reminder or alarm. An appointment can be marked
"Privé", as in Outlook: it shows a lock in the calendar, and the mark travels to Exchange and Microsoft 365 as
sensitivity Private, to Google as visibility private and to CalDAV as `CLASS:PRIVATE`, so anyone the calendar is
shared with there sees only that the time is taken. Categories, the same coloured labels as on mail, go on an appointment
in its window ("Categorie toevoegen", a chip with ✕ takes one off, "Alle categorieën wissen" empties the list); a
categorised appointment is painted in its first category's colour, as Outlook does. They travel to CalDAV as
`CATEGORIES`, to Exchange and Microsoft 365 as the item's categories, and to Google (which has none) as a private
extended property. "Weergeven als" in the same window sets how the time counts for
your availability, Outlook's Vrij, Voorlopig, Bezet or Afwezig: a free appointment is drawn as a pale block with the
calendar's colour as its border, a tentative one half-transparent, a busy one solid and an out-of-office one purple.
The status travels as Exchange's LegacyFreeBusyStatus, Microsoft 365's showAs, Google's transparency (free or busy,
all that Google knows) and iCalendar's `TRANSP` with Outlook's `X-MICROSOFT-CDO-BUSYSTATUS`. The contacts module keeps a local address
book (`contacts.json`) with search and a contact card as Outlook has it (photo, quick buttons to mail, chat and call, tabs
Contactpersoon and Organisatie with phones by kind, work and home addresses, job title, department, website and birthday;
photos come from Microsoft 365 and vCards, are kept only on this computer and are never loaded from a link); for a Microsoft 365 account the card also says whether the person is free now,
their working hours and the time where they are, from Graph's `getSchedule`, and the compose window completes
addresses from it and from the mail you have seen. A sender goes into the address book in one click: "Toevoegen aan
contacten" next to the sender in the reading pane (and "Afzender toevoegen aan contacten" in the message's right-click
menu) opens the contact form with the name and address filled in; when the address is already known the button reads
"Contact openen" and shows that card. Contacts travel as vCard files, as with Outlook and phones: "Opslaan als vCard" on a
card writes a `.vcf` to your Downloads folder, and "Importeren…" on the ribbon reads one or more `.vcf` files into the
local address book, skipping a card whose address is known already. The tasks module keeps a local to-do list
(`tasks.json`) with due days, priorities, a repeat like Outlook's "Terugkeerpatroon" (daily, weekly, monthly or yearly, every so many; finishing such a task adds the next one with the following due day, for tasks kept on this computer), an open/done filter and a search box; under the tasks it lists the flagged
mail with its follow-up day, as Outlook's To-Do list does (a click opens the message, the flag icon takes the flag off). The calendar has a search box
too: while it holds words, a list of the matching appointments of the past year and the two years ahead
(title, location or notes; one row per occurrence of a series) replaces the view, and a click opens the
appointment. An appointment can be dragged to another day in the month view, or to another day and half
hour in the week and day views; it keeps its length and everything else, in an account's calendar the move
goes to the server, and a repeating series is changed through its window instead. The views are Dag, Werkweek
(Monday to Friday, as in Outlook), Week and Maand; "Weeknummers" on the Beeld tab puts the ISO week number left of
each week in the month view and in the time gutter of the week and day views, as Outlook's "Weeknummers weergeven"
(off by default, kept in `view.json`); "Ga naar datum" in the ribbon (Ctrl+G) asks for a day as dag-maand-jaar and
shows it in the current view, as Outlook's Ga naar datum; "Werkuren" on the Beeld tab (on by default) shades the hours
outside 8:00-17:00 and the weekend in the day and week views, as Outlook shades the time outside the working day; each
calendar in the side pane stands under the address of its account (an account's own calendar is called "Agenda", and "Agenda (address)" where it stands alone, as in the appointment window), the ones on this computer under "Op deze computer", so two accounts with the same name stay apart, as Outlook groups them; contacts and tasks name their account by its address too. An account's other calendars on the server (a team calendar, birthdays, holidays, a calendar shared with you) stand under its "Agenda" with their own name and colour, for Microsoft 365, Google and CalDAV (Exchange EWS still shows the main calendar only); a calendar you may only read is not offered for a new appointment. Each calendar has a round dot in its colour, filled with a tick while it shows, and
clicking the row hides it: an unticked calendar keeps its appointments out of the views, the search list and the
printed page until it is ticked again (reminders still come), as in Outlook (kept in `view.json`). The "⋯" after a
calendar (or a right click) gives it one of twelve colours, each at least 3:1 against the window in both themes, and
renames or deletes a calendar on this computer; "Agenda toevoegen" makes a new local calendar with a name and colour.
A colour picked for an account's calendar stays on this computer (`calendar.json`). With more than one calendar ticked, the day and
week views show a tab per calendar in its colour, as Outlook does: merged, the calendars share one grid and
appointments at the same time stand side by side (each in its own colour); the arrow on a tab puts the calendars
side by side, each with its own days under its tab (a click in a calendar's hours makes the appointment there), and
back; the cross on a tab unticks that calendar. The choice is kept in `view.json`. In the week and day views
the bottom edge of an appointment can be dragged too, to make it longer or shorter by the half hour. The calendar's
side pane lists the open tasks; drag one onto a day or an hour and the appointment window opens for it, with the
task's title and notes filled in, so planning a task is one drag and "Opslaan". The appointment window also has
"Genodigden": each address becomes a chip on that line (with "Optioneel" for a second line of optional attendees),
and the button reads "Verzenden", which saves the appointment as a meeting and mails each attendee a meeting request
(the appointment as a `text/calendar` attachment with the organiser and attendees) from the calendar's account, as
Outlook does. Answers (accepted, tentative, declined, a new time proposed) are filed in the meeting by themselves:
inbox mail that reads like an answer is fetched, and its iTIP `REPLY` or `COUNTER` updates the attendee, but only when
it comes from that attendee for the current version of the meeting. The chips then show ✔, ? or ✕, the window counts
the answers and offers a proposed time; the answer itself shows as "Sanne heeft geaccepteerd". A meeting that changed
sends "Update verzenden" (a new time asks everyone again), people taken off get a cancellation, and "Vergadering
annuleren" calls it off for everyone. The attendees travel to CalDAV with `SCHEDULE-AGENT=CLIENT`, so a scheduling
server does not mail them twice; meetings from Exchange and Microsoft 365 bring their attendees and answers along. An
appointment that is over says "Deze gebeurtenis vindt plaats in het verleden.". "Doorsturen…"
in the window of a saved appointment opens a new message with the appointment in words and as a `text/calendar`
attachment (`METHOD:PUBLISH`, no organiser or attendees), so someone else can put it in their own calendar, like
forwarding an appointment in Outlook. In all three modules Ctrl+E goes to the search box and Esc clears it, as in the mail module. Contacts can be gathered in a
contact group ("Nieuwe groep" in the contacts module, like Outlook's Contactgroep): a name for a set of addresses, listed
under the contacts, with "E-mail aan groep" on its card; typing the name in an address field offers the whole group and
puts every member's address in the field. Groups live in `contact_groups.json` on this computer only. An Exchange, Microsoft 365 or Gmail account
brings its own calendar, contacts and tasks along (Google Calendar, People and Tasks for Gmail), and so
does an IMAP account with a CalDAV/CardDAV server next to it (Fastmail, iCloud, Nextcloud and the like): they
appear next to the local ones, changes go straight to the server, and a new item asks whether
it belongs on this computer or in the account. When that calendar server cannot be reached, the mail still connects and the account's card says what went wrong with the calendar. A mail server that does not answer, or a port with the wrong choice between TLS and STARTTLS, gives a plain error within a minute instead of "Verbinden…" without end. Everything an account showed is kept in an
encrypted offline cache, so the window fills at once on the next start and mail you opened
before can be read without a connection, and changes you make without one (flags, moves,
appointments, contacts, tasks) wait and reach the server when it is back, even after a restart.

Folders are yours to arrange: the "Map" tab in the ribbon (or a right click on a folder) makes
a new folder under the selected one, renames an ordinary folder or removes it with everything in
it, after asking; the inbox and the other special folders stay as they are. The change goes to
the server at once (IMAP `CREATE`/`RENAME`/`DELETE`, Exchange Web Services, Microsoft Graph) and
the folder pane shows the list the server answers with, so it needs a connection. The same tab
and menu empty a folder ("Map leegmaken", after asking: everything in it goes for good, on the
server too) and mark all its mail read. "Toevoegen aan Favorieten" (same tab and menu) pins a folder
of any account under "Favorieten" at the top of the folder pane, as Outlook has it; the choice is kept
in `view.json`. With more than one account, "Alle accounts" above the accounts lists Postvak IN,
Concepten, Verzonden items, Archief, Ongewenste e-mail and Verwijderde items once, each showing the
mail of every account together with the unread counts added up, as in Outlook; it is a view, so a
reply goes out from the account the message came to and a delete, move or drop stays in that
account. The group folds shut with its chevron, and so does each account. Subfolders sit a step in
under their folder, by their own name, as the server nests them (IMAP's hierarchy delimiter, Exchange's
parent folder, Microsoft Graph's child folders); a folder with subfolders starts folded shut, and what
is folded open is kept in `view.json`. Deleting a message that is already in "Verwijderde
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

The compose window is laid out as Outlook's: Van (a list with several accounts), Aan with CC,
BCC and the address book at its end, Onderwerp with "Urgentie" at its end, and Outlook's
formatting bar over the text: undo and redo, paste, typeface (Inter, DejaVu Serif, DejaVu Sans
Mono, all carried by the app) and size, clear formatting, text colour, highlight, bold, italic,
underline, bullets and numbering, and under "⋯" strikethrough, indentation and alignment, with
Ctrl/⌘+B, I, U, Z and Y. A formatted message goes out as HTML with the text beside it; the
HTML is written by the app itself from the formatting, with every character of the text escaped,
so nothing typed or pasted becomes markup. A message with blind copies (BCC) names them only in
the envelope. A few seconds after each change the window keeps the message under Concepten and
says "Concept is zojuist opgeslagen". The same bar sits over the notes of an appointment, a contact and a task; the formatting is kept
on this computer, servers get the notes as text, and a sync keeps the formatting as long as the
text comes back unchanged.
"Verzenden" closes the window at once, as in Outlook, and the message goes out behind it: it stays
under Concepten until the server has it, the notice at the top says "Bericht verzonden.", and when
sending fails the window comes back with the reason.

Templates are saved pieces of text, as Outlook's "Mijn sjablonen": the "Sjablonen" menu in the
compose window drops one into the message above the signature and the quoted text, "Tekst
opslaan als sjabloon…" keeps what you wrote so far under a name, and "Sjablonen beheren…" opens
the window to change or remove them. They are kept in `templates.json`.

Categories, the coloured labels Outlook users know, go on a message through "Categoriseren" in
the ribbon: Outlook's six to begin with, and "Alle categorieën…" adds, recolours or removes
them (`categories.json`). The list shows a coloured square per category, the open message its
chips (a click takes one off). The names travel with the mail: Exchange and Microsoft 365 keep
them as the item's categories, IMAP as keywords (spaces become `_`), so other clients see them
too; a category another client set shows in grey until you give it a colour here.

The "Beeld" tab chooses what the list shows and how: only unread, flagged or mail with
attachments, or only mail with a chosen category; sorted newest or oldest first, by sender or by subject; the reading pane to the
right or below the list, or off, in which case the list takes the width (Enter or a double-click on a message
opens it in its own window, as in Outlook, with any reading pane setting); "Gelezen markeren", whether a message opened
in the reading pane counts as read at once, after five seconds or not by itself, as Outlook's reading-pane options; compact rows without the preview line; "Gesprekken", which folds the messages on one
subject (whatever their "Re:" or "Fwd:" prefixes) behind the one that stands first in the list, with an arrow to
unfold them; and a light, dark or system theme. "Groepen" (on by default) heads the list with "Vandaag", "Gisteren", "Deze week", "Vorige week", "Deze maand", "Vorige maand" and "Ouder" while it is sorted by date, as Outlook does. The choices are kept in `view.json` next to the accounts file.

"Afdrukken" under "Bestand" (or Ctrl+P) prints the open message by way of the system browser: Merel writes a
page of its own (sender, recipients, date, subject, attachment names and the body as it drew it,
with embedded pictures and without anything from the internet or any script) to one file in the
temporary folder and opens it, and the browser's print dialog does the rest. In the calendar the same "Afdrukken"
(or Ctrl+P) prints the open view, as Outlook does: the day, work week, week or month as a list of days with each
appointment's time, title and place. "Opslaan als…" under "Bestand" (or Ctrl+S) saves the open message as an
.eml file in the Downloads folder, like Outlook's "Save As": the message as it arrived, headers and all, when the
cache still holds it, else a plain copy of its envelope and text; the file is named after the subject and a taken
name gets " (2)". A notice says where it went.

What an action has to say appears the way the new Outlook shows it: a message bar at the top of the window, just
under the ribbon, where the person was looking. Its icon and edge tell the kind apart, and a screen reader hears it
too: green with a check for done, blue for information, amber with a triangle for a warning, red only for what
failed. Good news and information go away after six seconds and a warning after twelve (never while the pointer
rests on it); an error stays until it is closed. A saved file is named by itself and its folder, not by its full
path, and "Toon in Finder" (Verkenner on Windows, the folder on Linux) shows it.

A flag can carry a date, Outlook's "Opvolgen": the right-click menu's "Opvolgen" offers Vandaag, Morgen,
Deze week (this week's Friday), Volgende week or Geen datum. The flag goes to the server as any flag does; the
date stays on this computer (`follow_ups.json`, filed under the message's Message-ID so a move keeps it). Over
the open message a strip names the day, turns red once it has passed, and "Voltooid" takes flag and date off;
the hover text of a row in the list names the day too.

A newsletter (mail with a `List-Unsubscribe` header) shows "Afmelden…" over the message, as Outlook does. Nothing
happens by itself: a window first says what will be done, a mail to the list's address from the account the message
came in on (preferred, since it tells the sender nothing new) or the list's web page in the browser (which does tell
the sender the mail was read), and the person decides. Once the request is out, the message goes to the Junk folder
(and the junk filter learns from it); a request that failed leaves it where it is. Mail in the Junk folder gets no such offer.

Read receipts, Outlook's "Leesbevestiging": "Leesbevestiging vragen" in the compose window adds the
`Disposition-Notification-To` header, and a message whose sender asked for one shows "De afzender vraagt een
leesbevestiging." over the text with "Leesbevestiging sturen". Nothing goes back by itself: the receipt is a short plain
mail ("Gelezen: subject", with the moment it was read, answering the message) from the account the message came in
on, sent only when the button is pressed; the sender learns nothing otherwise. Mail in the Junk folder gets no such
offer.

Digitally signed mail (S/MIME), as in Outlook: on an account's card under Bestand, "Certificaat importeren…" reads a
.p12 or .pfx file with its password (from your organisation or a certificate authority; Triple DES exports from Windows
work too). The certificate must be issued for the account's address and meant for e-mail. From then on new mail from
that account goes out signed (`multipart/signed`, SHA-256 with RSA, the certificates enclosed so the reader can check
the signature and later answer encrypted); the "Ondertekenen" box in the compose window turns it off for one message.
A message that should be signed and cannot be (an expired certificate, say) is not sent unsigned: the window says why.
The private key never lies on disk in the clear: it is sealed with ChaCha20-Poly1305 under the key of the offline
cache, bound to the account, in `cache/<account>.identity`, and goes when the certificate or the account is removed.
Signing is aws-lc-rs; the CMS structures come from the RustCrypto `cms` crate. Signed mail that comes in is checked
when it opens (RSA with SHA-256/384/512, ECDSA P-256 and P-384): a strip over the message says "Digitaal ondertekend
door …" when the signature holds, warns when the certificate belongs to another address than the sender's, and warns
in red when the message changed after it was signed; a signature Merel cannot check (SHA-1, an unknown algorithm)
is named as such, never shown as valid. The `smime.p7s` part is not listed as an attachment. The certificate's issuer
is checked too: the chain from the certificate through the issuers enclosed with the signature must reach one of the
computer's trusted root certificates (the system store, as for TLS), each link signed by the next, each issuer a
certificate authority, all valid now. A valid signature from another issuer (a self-made certificate, or a company's
own CA) gets a warning that anyone can make such a certificate, with "Certificaat vertrouwen" to accept it for that
address; revocation is not checked online.

Encrypted mail (S/MIME), Outlook's "Versleutelen": with a certificate on the account the compose window has a
"Versleutelen" box. The message and its attachments go out as `application/pkcs7-mime` (AES-256-CBC, the key wrapped
with RSA for every recipient and for the sender, so Sent stays readable); the subject travels in the clear, as in
every mail program. Merel learns a correspondent's certificate the way Outlook does, from a valid signature on
their mail that matches the sender and comes from a trusted issuer (not from the Junk folder; others only through
"Certificaat vertrouwen"), and keeps it in `certificates.json`; encrypting to
someone whose certificate is not known yet is refused with their address named, never sent in the clear. Encrypted
mail that comes in is opened with the account's certificate (RSAES-PKCS1-v1_5 or OAEP, AES-128/192/256) and shows a
"🔒 Versleuteld bericht" strip; one that cannot be opened says why in place of its text. The offline cache keeps it
encrypted as it came.

OpenPGP, for people who use GnuPG, Thunderbird or Proton: on an account's card under Bestand, "OpenPGP-sleutel
importeren…" reads an exported private key (`gpg --export-secret-keys --armor`, .asc) with its passphrase; one of its
addresses must be the account's. The key is kept unlocked but sealed like the S/MIME key, in `cache/<account>.pgpkey`.
PGP/MIME mail that comes in is read with rPGP: an encrypted message (`multipart/encrypted`, GnuPG 2.4's OCB packets
included) opens with the account's key and shows the "🔒 Versleuteld bericht" strip; a signature, made as PGP/MIME or
inside the encryption, gets a strip saying whose key made it, a red one when the message changed. Correspondents' keys
come the Autocrypt way: a message whose `Autocrypt` header carries a key for its sender's own address teaches it, once
(not from the Junk folder), and it is kept in `pgp_keys.json`; a later, other key for the same address is not taken
over by itself. Mail from an account with an OpenPGP key carries it in an `Autocrypt` header and goes out signed
(PGP/MIME `multipart/signed`), unless Ondertekenen is off for the message; Versleutelen encrypts it for the recipients
and the sender (`multipart/encrypted`, signed inside, AES-256), once every recipient's key is known. An account with
both an S/MIME certificate and an OpenPGP key signs with S/MIME, and encrypts with OpenPGP only when that reaches every
recipient and S/MIME does not.

Chat over e-mail, the way Delta Chat does it, so a chat needs no server of its own and the other side can use Delta
Chat on any device: a chat message is an ordinary message with a `Chat-Version: 1.0` header (and `Chat-Group-ID` and
`Chat-Group-Name` for a group), which the IMAP listing fetches with the other headers. The engine makes chats from
those messages, one per group and else one per person, with the copy in Verzonden items counted once and nothing from
Junk or Verwijderde items. Turning chat on for an account keeps its OpenPGP key or makes one (Ed25519 with a
Curve25519 subkey, as Delta Chat makes them), which from then on goes along with its mail in the `Autocrypt` header.
A chat message only ever goes out encrypted and signed with OpenPGP; to someone whose key Merel does not have yet it
does not go out at all, and Merel says to send an invitation first. The Chat module, between Agenda and Personen,
lists the chats with search and the chips Alle, Ongelezen and Groepen, shows a conversation with one's own messages on
the right and a line to type in, and starts a chat with one person or a group. Chat messages stay out of the mail
folders' lists.

Delta Chat keeps everything but the address inside the encryption (protected headers, RFC 9788): outside a message
shows `[...]` as its subject and a date up to a week off, and no chat header at all. So for an account with chat on,
Merel opens the newest OpenPGP-encrypted messages after each listing, and the header inside takes the outer one's
place: the subject, the real date, the sender's name and the chat tag; the next listing keeps it. A message also
brings its sender's key inside (`Autocrypt`), and in a group the others' keys (`Autocrypt-Gossip`); Merel keeps the
sender's like any Autocrypt key, and the others' only from a message signed with a key it already believes. A signature
that only checks out with a key the message brought along counts for nothing when Merel knows another key for that
address. Going out, every encrypted message carries its header inside too, and a group chat message tells everyone
everyone's key; `Secure-Join` fields and gossip travel inside only.

To start a chat with someone whose key Merel does not have, there is an invitation link, Delta Chat's own
(`https://i.delta.chat/#…`, with the key's fingerprint, the address and two random tokens), shown with its QR code on
the Chat start page: copy it, mail it, or make a new one, which makes the old one useless. Someone with Delta Chat
scans the code or opens the link; in Merel one pastes a link or `OPENPGP4FPR:` code under "Uitnodiging aannemen".
Then the two sides run Delta Chat's Secure-Join handshake by mail: the first step goes out unencrypted with only the
invitation number, every next one encrypted and signed, and each side checks that the key it gets matches the
fingerprint in the link and that the secret token came back. At the end both sides keep the other's key as verified
and the chat header says "geverifieerd". The tokens and verified keys stay on this computer (`invites.json`); a
handshake message that does not fit gets no answer. Group invitations are not taken yet.

The Vergaderen module, between Chat and Personen, lists the online meetings of today or the coming seven days: every
appointment with a link to join. Microsoft 365 and Google name the link with the event (`onlineMeeting.joinUrl`,
`hangoutLink` or the video entry of `conferenceData`), Outlook's and Google's invitations in an iCalendar property of
their own (`X-MICROSOFT-SKYPETEAMSMEETINGURL`, `X-GOOGLE-CONFERENCE`); otherwise Merel takes the first link in the
location or the notes to a service people meet with (Microsoft 365, Zoom, Google Meet, Jitsi Meet, Webex, Whereby). Only
an `https` link without spaces or a user name before the host counts. A meeting shows how long until it starts, its
organiser and attendees with their answers, and from a quarter of an hour before the start a "Deelnemen" button, which
opens the link in the browser (the service's own app takes over when it is installed). A link someone sent can be pasted
on the start page to join, and "Vergadering plannen" opens a new appointment.

A message can be put off, Outlook's "Sluimeren": the right-click menu's "Sluimeren" offers Later vandaag (three
hours on), Morgen, Dit weekend (Saturday) or Volgende week (Monday), at eight in the morning. The message leaves the
list until then and comes back unread, with a notice; the "Gesluimerd" filter on the Beeld tab shows
what is put off, a strip over such a message names the moment, and "Niet meer sluimeren" brings it back now. The list
stays on this computer (`snoozed.json`, filed under the Message-ID so a move keeps it); the server only sees the
unread flag when the message returns.

Several messages can be selected at once, as in Outlook: Ctrl+click (Cmd on a Mac) adds a message to the
selection or takes it out, Shift+click selects the run from the open message to the clicked one and Ctrl+A
takes the whole list. The reading pane then says how many are selected, and Delete, Archive, Junk, Move,
Mark read or unread and Flag act on all of them.

Keyboard shortcuts in the mail module follow Outlook (Cmd instead of Ctrl on a Mac): arrow keys
walk the message list, Delete removes the selected message, Backspace archives it (on a Mac, where Backspace is
the delete key, Ctrl+E archives, as in Outlook for Mac), Ctrl+N starts a new message, Ctrl+R
replies, Ctrl+Shift+R replies to all, Ctrl+F forwards, Ctrl+Shift+V moves to a folder, Ctrl+Q and Ctrl+U mark read or unread,
Ctrl+A selects every message in the list, Ctrl+E puts the cursor in the search box, Ctrl+P prints, Ctrl+S saves the message as an .eml file, Ctrl+Shift+1 to 9 run the account's quick steps, Esc clears a
search, F5 refreshes the folder and F9 syncs every account, as Outlook's Send/Receive. The same sync sits as a button over
the message list and on the Send/Receive tab; a right click on an account's heading syncs only that account: it lists
its folders again, fetches its inbox, every folder whose mail is listed, and its calendar, contacts and tasks, and an
account that lost its connection tries the server at once. A right click on that button, or the menu next to it on the
Send/Receive tab (on a Mac: Map > Automatisch synchroniseren), chooses how often Merel syncs by itself: every 1, 2 (the
start), 5, 10, 15, 30 or 60 minutes, or only by hand; the calendar, contacts and tasks follow at most every five minutes,
and chats keep coming every two. From every module, Ctrl+Shift+M, Ctrl+Shift+A, Ctrl+Shift+C and Ctrl+Shift+K start a
new message, appointment, contact or task, as in Outlook (the contact and task forms open in their own module). None
of them fires while you are typing in a text field or writing a message. A right click on a message opens a menu with
the everyday actions (reply, forward, read or unread, flag, move, archive, junk, delete) and two Outlook favourites:
"Taak maken van dit bericht" and "Afspraak maken van dit bericht", which open a new task or appointment named after the
message, with the sender, the date and the text as notes. "Beantwoorden met vergaderverzoek" (also Ctrl+Alt+R) does the same with the sender and the other recipients already filled in as "Genodigden", so
one "Verzenden" invites everyone on the message, as in Outlook. Two more are about the sender: "Alles van deze afzender
zoeken" puts the address in the search box over every folder, and "Regel maken voor deze afzender…" opens the rules
window with a rule for that address filled in. Hovering a ribbon button
shows its shortcut, and the "Help" tab lists them all in one window; the same tab opens the folder
that holds your settings and cache, the project page, and "Over Merel" with the version. The version names the commit the app was
built from and its date (for example "Versie 0.1.0 (build 287797c van 2026-10-20)"), shown at the end
of the Help tab and in "Over Merel", where "Kopiëren" puts it on the clipboard for a bug report.

## Readability

Every colour pair the window draws meets WCAG 2.2 level AA: text at 4.5:1 or better on each
surface it can sit on, including captions, previews, placeholders and disabled controls (AA
exempts the last two; Merel does not), and icons and control borders at 3:1 or better, in
the light and the dark theme. `theme::contrast` measures the ratios and a test holds both themes
to them, so a colour change that drops below the bar fails the build.

## Languages

Merel speaks Dutch and English. It follows the language the computer is set to (Dutch for Dutch,
English for every other), and View › Language picks one for good, without a restart; the welcome
wizard offers the same choice. Every word on screen is translated: the ribbons and the macOS menu
bar, mail, the compose window, calendar, contacts, tasks, the wizards and settings, notifications
and the error messages from the servers, certificates and keys. Dates use the month and day names
of the language. The standard folders take Outlook's names in that language (Postvak IN or Inbox,
Verwijderde items or Deleted Items) and so does the calendar every installation starts with
(Mijn agenda or My calendar); names you chose yourself stay as they are. Replies and forwards
quote their header in the language Merel speaks.

The words live in one [Fluent](https://projectfluent.org) file per language,
`crates/i18n/locales/nl.ftl` and `en.ftl`, built into the binary by the `merel-i18n` crate that
every layer uses; a text missing from one falls back to English. To add a language, copy
`en.ftl`, translate the right-hand side of each line and add the language to
`crates/i18n/src/lib.rs`; a test checks that every file has every text, and the ribbon tests check
that the buttons still fit the window in every language.
Tests always run in Dutch, whatever the machine is set to.

## Fonts

The window draws its text with [Inter](https://github.com/rsms/inter) (SIL Open Font License) and
its arrows and shapes with DejaVu Sans (Bitstream Vera licence); both are embedded in the binary
from `crates/ui-egui/assets/fonts`, where their licences sit next to them. egui's own emoji fonts
stay as the fallback for the pictographs.

## Build

```
cargo run -p merel
cargo test
cargo clippy --all-targets -- -D warnings
```

Requires Rust 1.95 or newer.

See [`docs/architecture.md`](docs/architecture.md).

## Install

Each release on the [releases page](https://github.com/alainrees/merel/releases) carries an
installer per platform, built by `.github/workflows/release.yml` when a tag `vX.Y.Z` is pushed
(the tag must match the version in `Cargo.toml`); `SHA256SUMS.txt` lists their checksums.

| Platform | File | How |
| --- | --- | --- |
| macOS 11+ (Apple Silicon and Intel) | `Merel-X.Y.Z-macos.dmg` | Drag Merel to Applications. The app has a free ad-hoc signature, not an Apple one, so the first start needs "Open Anyway" under System Settings > Privacy & Security. |
| Windows 10+ (x64) | `Merel-X.Y.Z-windows-setup.exe` | A short wizard in Dutch or English; installs for the current user without administrator rights. SmartScreen asks once, under "More info". |
| Linux (x86_64) | `Merel-X.Y.Z-linux-x86_64.AppImage` or `merel_X.Y.Z-1_amd64.deb` | Make the AppImage executable and run it, or install the .deb on Debian or Ubuntu. |

The first start on a computer opens "Welkom bij Merel", four short steps that can be left at
any point: what Merel is, adding an account (opens Bestand), whether to check daily for a new
version, and light or dark. It comes once; every choice stays changeable in its usual place.

On macOS the app menu is Merel's own, in Merel's language: "About Merel" (Over Merel) opens the
window with the version and build, "Check for updates…" checks at once, and Verberg/Stop Merel carry the app's
name (built with [muda](https://crates.io/crates/muda); there is no Edit menu, so Cmd+C and Cmd+V
stay with the text fields).

Merel tells you when a newer release is out: "Updates" on the Help tab asks GitHub right
away, and "Elke dag controleren op een nieuwe versie" in "Over Merel" asks once a day (off
until you switch it on). A newer version opens a small window with its release notes,
"Bijwerken", "Deze versie overslaan" and "Later"; the question carries no account or identifier.

"Bijwerken" is a short wizard: it downloads the installer for this platform from the release,
checks it, installs it and offers "Opnieuw starten". On macOS the `.dmg` is mounted read-only
and its `Merel.app` swapped in for the running one; on Windows the installer runs silently and
starts Merel again; on Linux a new AppImage replaces the running one. Nothing is opened or run
before two checks pass: the release's `SHA256SUMS.txt` carries an Ed25519 signature
(`SHA256SUMS.txt.sig`) under the key in `crates/ui-egui/src/install.rs`, and the download
matches its line in that list. When Merel cannot replace itself (run with `cargo`, installed
from the `.deb`) or a check fails, the button opens the release page instead.

The release key is made once with `cargo run -p merel-sign -- keygen`: the secret half goes into
the repository's Actions secret `MEREL_SIGNING_KEY` (the release workflow signs with it), the
public half into `SIGNING_KEY` in `install.rs`. A release without the secret is published
unsigned, and the app then offers only the release page.

The packaging files live in `packaging/`: the app icon, `macos/bundle.sh` with `Info.plist`,
`windows/merel.iss` for Inno Setup and `linux/` with the desktop entry and the AppImage script.

## iPhone and Android

The phone apps are on their way: native screens (SwiftUI on the iPhone, Jetpack Compose on Android) on top of the same engine as the desktop. `crates/mobile` puts that engine behind a UniFFI interface; the `Mobile` workflow builds it for both phones and writes the Swift and Kotlin bindings. The iPhone app is in `apps/ios` (SwiftUI): sample mail, adding an IMAP/SMTP account, folders, the message list with swipe actions, and reading; see its README for running it on your own iPhone. The Android app is in `apps/android` (Jetpack Compose) with the same mail screens; CI builds it as an APK.

## License

Merel is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option. Unless you state otherwise, any contribution you submit
for inclusion in Merel is dual licensed as above, without any additional terms or conditions.
