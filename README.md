# Mailcraft

Mail and calendar for the desktop, with the Outlook workflow you already know and the look of the
ArtCraft family (WordCraft, GridCraft, DeskCraft). Pure Rust, one binary, runs on macOS, Windows
and Linux.

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
two minutes (calendar, contacts and tasks every five), the status bar counts new unread messages,
the window asks for attention, and its title shows the unread mail in the inboxes of every account
together ("Mailcraft (3)"), so the count is visible in the dock, taskbar or window switcher. Junk: the
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
one of your own addresses. The lists live on your computer; nothing about your mail is sent anywhere to be checked.
"Gesprek negeren" (in a message's right-click menu, or Ctrl+Delete) is Outlook's Ignore: the messages of
that conversation (same subject, prefixes aside) in the inbox go to "Verwijderde items", and so does every message
that arrives in it afterwards; a strip above such a message has "Niet meer negeren", which brings them back. The
list is `ignored.json`, per account. "Gesprek opruimen…" (same menu) and "Map opruimen…" (a folder's menu, or
"Opruimen" on the Map tab) are Outlook's Clean Up: of each conversation the older messages go to "Verwijderde
items" and the newest stays, as do unread, flagged and categorised messages and messages with attachments; Mailcraft
asks first and says how many would go. It goes by the headers it has, not by whether a later reply quotes the whole
earlier message. Phishing: a message that shows signs of it
gets a warning strip above its text that names them, with "Afzender blokkeren" at hand: a sender who uses a
contact's name (or an address as a name) with another address, a domain one letter or one look-alike letter away
from a domain you know (your contacts', your own), failed SPF/DKIM/DMARC checks reported by your mail server in the
`Authentication-Results` headers, a link whose text shows one site while it goes to another, and an attachment that
runs as a program. The checks run on your computer only and never decide for you. In the Junk folder itself a
message is shown with its links, attachments and pictures from the internet switched off, as Outlook does; "Geen
ongewenst" brings it back with everything working. A search box above the
message list narrows it by subject, sender, recipient or preview text, in the current folder or
across every folder of every account (it searches what Mailcraft has fetched so far). Outlook's search words work in it:
`van:sanne`, `aan:jan` and `onderwerp:offerte` look in one field, `heeft:bijlage`, `is:ongelezen` and `is:gemarkeerd` keep
only such messages (the English `from:`, `to:`, `subject:`, `has:attachment`, `is:unread` and `is:flagged` too), and the
rest are plain words that may occur anywhere. "Zoek op de server" asks the mail server itself, which also looks in older mail and in the text of the messages; the server gets the plain words, and what it finds is then held to the same search words here. "Zoekmap" next to the box saves the search under a name, as an Outlook search folder: it is listed under "Zoekmappen" in the folder pane with its words, scope and list filter, a click runs it again, and its menu removes it (`view.json`). The 🕒 menu next to the box lists the last ten searches, as Outlook's "Recente zoekopdrachten": a click searches those words again, "Lijst wissen" forgets them (`view.json`). Sending over SMTP: new mail,
reply, reply all and forward (a forwarded message takes its attachments along, as in Outlook, except from the Junk folder; "Doorsturen als bijlage" in the message menu, or Ctrl+Alt+F, sends the message itself along as an .eml file with its headers, like Outlook's "Forward as Attachment", not from the Junk folder either; "Opnieuw verzenden" opens a message, a sent one say, as a new message with the same recipients, subject, text and attachments, like Outlook's "Resend This Message"), with attachments (picked with the file dialog or dropped onto the window, 25 MB together), a copy filed in Sent and the account's signature under the text. Closing a message you were writing puts it aside as a draft ("Concepten (deze computer)" under the account, kept in the encrypted offline cache, never on the server) to open, finish and send later. "Later verzenden" in the same window puts the message aside with a moment (in an hour, this evening, tomorrow morning, Monday morning), like Outlook's "Bezorging uitstellen": it shows under Concepten with that moment, Mailcraft sends it then if it is running, and opening it lets you drop the moment or send at once; one that cannot go becomes an ordinary draft again with a notice. Before a message goes, Mailcraft asks once, as Outlook does, when the text speaks of an attachment ("bijlage", "bijgevoegd", "attached", …) while none is on, or when the subject is empty; "Toch verzenden" sends it as it is. A password account's name,
servers, user name and password can be changed afterwards from the account list. Gmail and Microsoft 365 sign in through
the system browser (OAuth 2.0 with PKCE; XOAUTH2 on IMAP and SMTP for Gmail, Microsoft Graph
for Microsoft 365). Microsoft 365 uses Mailcraft's own Entra registration, so only name and address are asked,
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
book (`contacts.json`) with search and a contact card, and the compose window completes
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
calendar under "Mijn agenda's" in the side pane has a round dot in its colour, filled with a tick while it shows, and
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
"Genodigden": with addresses there the button reads "Verzenden", which saves the appointment, notes who was
invited and mails each of them a meeting request (the appointment as a `text/calendar` attachment with the organiser
and attendees) from the calendar's account, as Outlook does; replies are ordinary mail and nothing is tracked. "Doorsturen…"
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
in `view.json`. Deleting a message that is already in "Verwijderde
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

"Afdrukken" under "Bestand" (or Ctrl+P) prints the open message by way of the system browser: Mailcraft writes a
page of its own (sender, recipients, date, subject, attachment names and the body as it drew it,
with embedded pictures and without anything from the internet or any script) to one file in the
temporary folder and opens it, and the browser's print dialog does the rest. In the calendar the same "Afdrukken"
(or Ctrl+P) prints the open view, as Outlook does: the day, work week, week or month as a list of days with each
appointment's time, title and place. "Opslaan als…" under "Bestand" (or Ctrl+S) saves the open message as an
.eml file in the Downloads folder, like Outlook's "Save As": the message as it arrived, headers and all, when the
cache still holds it, else a plain copy of its envelope and text; the file is named after the subject and a taken
name gets " (2)". The status bar says where it went.

A flag can carry a date, Outlook's "Opvolgen": the right-click menu's "Opvolgen" offers Vandaag, Morgen,
Deze week (this week's Friday), Volgende week or Geen datum. The flag goes to the server as any flag does; the
date stays on this computer (`follow_ups.json`, filed under the message's Message-ID so a move keeps it). Over
the open message a strip names the day, turns red once it has passed, and "Voltooid" takes flag and date off;
the hover text of a row in the list names the day too.

A newsletter (mail with a `List-Unsubscribe` header) shows "Afmelden…" over the message, as Outlook does. Nothing
happens by itself: a window first says what will be done, a mail to the list's address from the account the message
came in on (preferred, since it tells the sender nothing new) or the list's web page in the browser (which does tell
the sender the mail was read), and the person decides. Mail in the Junk folder gets no such offer.

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
in red when the message changed after it was signed; a signature Mailcraft cannot check (SHA-1, an unknown algorithm)
is named as such, never shown as valid. The `smime.p7s` part is not listed as an attachment. The certificate's issuer
is checked too: the chain from the certificate through the issuers enclosed with the signature must reach one of the
computer's trusted root certificates (the system store, as for TLS), each link signed by the next, each issuer a
certificate authority, all valid now. A valid signature from another issuer (a self-made certificate, or a company's
own CA) gets a warning that anyone can make such a certificate, with "Certificaat vertrouwen" to accept it for that
address; revocation is not checked online.

Encrypted mail (S/MIME), Outlook's "Versleutelen": with a certificate on the account the compose window has a
"Versleutelen" box. The message and its attachments go out as `application/pkcs7-mime` (AES-256-CBC, the key wrapped
with RSA for every recipient and for the sender, so Sent stays readable); the subject travels in the clear, as in
every mail program. Mailcraft learns a correspondent's certificate the way Outlook does, from a valid signature on
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

A message can be put off, Outlook's "Sluimeren": the right-click menu's "Sluimeren" offers Later vandaag (three
hours on), Morgen, Dit weekend (Saturday) or Volgende week (Monday), at eight in the morning. The message leaves the
list until then and comes back unread, with a line in the status bar; the "Gesluimerd" filter on the Beeld tab shows
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
search and F5 refreshes the folder. From every module, Ctrl+Shift+M, Ctrl+Shift+A, Ctrl+Shift+C and Ctrl+Shift+K start a
new message, appointment, contact or task, as in Outlook (the contact and task forms open in their own module). None
of them fires while you are typing in a text field or writing a message. A right click on a message opens a menu with
the everyday actions (reply, forward, read or unread, flag, move, archive, junk, delete) and two Outlook favourites:
"Taak maken van dit bericht" and "Afspraak maken van dit bericht", which open a new task or appointment named after the
message, with the sender, the date and the text as notes. "Beantwoorden met vergaderverzoek" (also Ctrl+Alt+R) does the same with the sender and the other recipients already filled in as "Genodigden", so
one "Verzenden" invites everyone on the message, as in Outlook. Two more are about the sender: "Alles van deze afzender
zoeken" puts the address in the search box over every folder, and "Regel maken voor deze afzender…" opens the rules
window with a rule for that address filled in. Hovering a ribbon button
shows its shortcut, and the "Help" tab lists them all in one window; the same tab opens the folder
that holds your settings and cache, the project page, and "Over Mailcraft" with the version. The version names the commit the app was
built from and its date (for example "Versie 0.1.0 (build 287797c van 2026-10-20)"), shown at the end
of the Help tab and in "Over Mailcraft", where "Kopiëren" puts it on the clipboard for a bug report.

## Readability

Every colour pair the window draws meets WCAG 2.2 level AA: text at 4.5:1 or better on each
surface it can sit on, including captions, previews, placeholders and disabled controls (AA
exempts the last two; Mailcraft does not), and icons and control borders at 3:1 or better, in
the light and the dark theme. `theme::contrast` measures the ratios and a test holds both themes
to them, so a colour change that drops below the bar fails the build.

## Fonts

The window draws its text with [Inter](https://github.com/rsms/inter) (SIL Open Font License) and
its arrows and shapes with DejaVu Sans (Bitstream Vera licence); both are embedded in the binary
from `crates/ui-egui/assets/fonts`, where their licences sit next to them. egui's own emoji fonts
stay as the fallback for the pictographs.

## Build

```
cargo run -p mailcraft
cargo test
cargo clippy --all-targets -- -D warnings
```

Requires Rust 1.95 or newer.

See [`docs/architecture.md`](docs/architecture.md).

## Install

Each release on the [releases page](https://github.com/alainrees/mailcraft/releases) carries an
installer per platform, built by `.github/workflows/release.yml` when a tag `vX.Y.Z` is pushed
(the tag must match the version in `Cargo.toml`); `SHA256SUMS.txt` lists their checksums.

| Platform | File | How |
| --- | --- | --- |
| macOS 11+ (Apple Silicon and Intel) | `Mailcraft-X.Y.Z-macos.dmg` | Drag Mailcraft to Applications. The app has a free ad-hoc signature, not an Apple one, so the first start needs "Open Anyway" under System Settings > Privacy & Security. |
| Windows 10+ (x64) | `Mailcraft-X.Y.Z-windows-setup.exe` | A short wizard in Dutch or English; installs for the current user without administrator rights. SmartScreen asks once, under "More info". |
| Linux (x86_64) | `Mailcraft-X.Y.Z-linux-x86_64.AppImage` or `mailcraft_X.Y.Z-1_amd64.deb` | Make the AppImage executable and run it, or install the .deb on Debian or Ubuntu. |

The packaging files live in `packaging/`: the app icon, `macos/bundle.sh` with `Info.plist`,
`windows/mailcraft.iss` for Inno Setup and `linux/` with the desktop entry and the AppImage script.

## License

Mailcraft is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option. Unless you state otherwise, any contribution you submit
for inclusion in Mailcraft is dual licensed as above, without any additional terms or conditions.
