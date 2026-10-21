# Merel in English.
#
# Each line is "key = text". The keys are the same in every language; translate only the text
# to the right of the = sign. See https://projectfluent.org for plurals and { $name }.

## View > Theme

theme-group = Theme
theme-system = System
theme-system-hint = Light or dark, as the computer is set
theme-light = Light
theme-light-hint = Light theme
theme-dark = Dark
theme-dark-hint = Dark theme

## View > Language

language-group = Language
language-menu-hint = The language of Merel
language-system = System language
language-system-hint = The language the computer is set to: Dutch, or else English
language-dutch-hint = Merel in Dutch
language-english-hint = Merel in English

## Modules en tabbladen

module-mail = Mail
module-calendar = Calendar
module-chat = Chat
module-meetings = Meetings
module-people = People
module-tasks = Tasks
tab-file = File
tab-home = Home
tab-folder = Folder
tab-send-receive = Send / Receive
tab-view = View
tab-help = Help
ribbon-not-yet = Not available yet

## Menubalk (macOS)

menu-file = File
menu-message = Message
menu-calendar = Calendar
menu-chat = Chat
menu-meetings = Meetings
menu-people = People
menu-tasks = Tasks
menu-folder = Folder
menu-view = View
menu-help = Help
app-about = About Merel
app-check-updates = Check for Updates…
app-services = Services
app-hide = Hide Merel
app-hide-others = Hide Others
app-show-all = Show All
app-quit = Quit Merel
app-window = Window
app-minimize = Minimize
app-zoom = Zoom
app-fullscreen = Enter Full Screen
app-bring-all-to-front = Bring All to Front
menu-accounts = Accounts…

## Knoppen die op meer linten staan

new-mail = New email
new-appointment = New appointment
new-contact = New contact
new-task = New task
delete = Delete
refresh = Refresh
edit = Edit

## Start-lint in E-mail

mail-reply = Reply
mail-reply-all = Reply all
mail-forward = Forward
mail-archive = Archive
mail-junk = Junk
mail-move = Move
mail-rules = Rules
mail-rules-hint = Move or mark mail automatically
mail-read = Read
mail-unread = Unread
mail-flag = Flag
group-new = New
group-respond = Respond
group-clean = Clean up
group-file = Organize
group-tags = Tags
categories-group = Categories
categorize = Categorize
categories-clear = Clear all categories
categories-all = All categories…
quick-steps = Quick steps
quick-steps-manage = Manage…
quick-steps-manage-hint = Create or change quick steps

## Start-lint in Agenda

calendar-today = Today
calendar-goto = Go to date
calendar-day = Day
calendar-work-week = Work week
calendar-week = Week
calendar-month = Month

## Start-lint in Contacten en Taken

contacts-new-group = New group
contacts-send-mail = Send email
contacts-import = Import…
contacts-import-title = Choose vCard files
tasks-reopen = Reopen
tasks-complete = Complete

## Map-lint

folder-new = New folder
folder-new-hint = A folder under the selected folder
folder-rename = Rename
folder-rename-hint = Give the selected folder another name
folder-delete = Delete folder
folder-delete-hint = Delete the selected folder and everything in it
folder-empty = Empty folder
folder-empty-hint = Delete every message in the selected folder for good
folder-all-read = All read
folder-all-read-hint = Mark every message in the selected folder as read
folder-clean-up = Clean up folder
folder-clean-up-hint = Move older messages of each conversation in the selected folder to Deleted Items
folder-favorite = Favorite
folder-favorite-off-hint = Take the selected folder out of Favorites
folder-favorite-on-hint = Put the selected folder under Favorites at the top of the folder pane

## Help-lint

help-shortcuts = Shortcuts
help-shortcuts-hint = Every key of the mail module at a glance
help-settings-folder = Settings folder
help-settings-folder-hint = Open the folder with accounts, calendar, rules and cache
help-github = On GitHub
help-github-hint = Open the project page with the manual in the browser
help-about = About Merel
help-about-hint = Version and principles
help-updates = Updates
help-updates-hint = Ask GitHub whether there is a newer version of Merel

## Beeld-lint

view-category = Category
view-all-categories = All categories
view-category-hint = Show only messages with this category
view-sort = Sort
view-sort-hint = Order of the message list
sort-newest = Newest first
sort-oldest = Oldest first
sort-sender = By sender
sort-subject = By subject
view-reading-pane = Reading pane
view-pane-right = Right
view-pane-right-hint = The reading pane right of the message list
view-pane-bottom = Bottom
view-pane-bottom-hint = The reading pane under the message list
view-pane-off = Off
view-pane-off-hint = No reading pane; Enter or a double click opens the message in a window of its own
view-mark-read = Mark as read
view-mark-read-menu = Mark read
view-mark-read-hint = { $menu }: when an opened message counts as read. Now: { $now }.
mark-read-now = At once
mark-read-later = After 5 seconds
mark-read-never = Not by itself
view-list = List
view-compact = Compact
view-compact-hint = Rows without the preview line, so more messages fit
view-conversations = Conversations
view-conversations-hint = Messages on one subject as one conversation behind the newest; the arrow in front shows the rest.
view-groups = Groups
view-groups-hint = Headings Today, Yesterday, This week, Last week, This month… over the messages, as in Outlook; only when sorted by date.
view-calendar = Calendar
view-week-numbers = Week numbers
view-week-numbers-hint = The week number left of each week in the month and week views, as in Outlook.
view-work-hours = Work hours
view-work-hours-hint = The hours outside 8:00-17:00 and the weekend grey in the day and week views, as in Outlook.

## Chat

chat-title = Chat
chat-new = New chat
chat-all = All chats
chat-search = Search chats
chat-filter-all = All
chat-filter-unread = Unread
chat-filter-groups = Groups
chat-none = No chats yet.
chat-not-found = No chat found.
chat-unread-label = { $title }, { $count } unread
chat-mine = You: { $text }
chat-encrypted = End-to-end encrypted
chat-loading = Fetching message…
chat-not-sent = Not sent: { $reason }
chat-input = Type a message
chat-input-hint = Type a message to { $name }…
chat-send = Send
chat-intro = Chat goes through your e-mail and is always end-to-end encrypted: only you and the other person can read it. They don't need Merel; Delta Chat works too, on any phone or computer.
chat-add-account = Add an e-mail account first to chat.
chat-imap-only = Chat works with IMAP accounts for now. Microsoft 365, Exchange and Gmail will follow.
chat-on = Chat is on for { $email }.
chat-off = Chat is off for { $email }.
chat-key-note = Merel then makes a key for this address. Mail from this account is signed from then on and carries your key, so others can write to you encrypted.
chat-enable = Turn on chat for { $email }
chat-start = Start
chat-cancel = Cancel
chat-pick-account = Choose an account
chat-via = Via { $email }
chat-to-hint = To: e-mail addresses, separated by commas
chat-name-hint = Group name (optional)
chat-first-hint = Type your first message…
chat-check-addresses = Check the addresses.
chat-ready-note = The chat is end-to-end encrypted. Someone you have no encrypted chat with yet first needs your invitation link; until then nothing is sent.
chat-enable-first = Turn on chat for this account first; you can do that on the Chat start page.
chat-invite = Invitation link
chat-invite-for = Invitation for { $email }
chat-invite-intro = Anyone who opens this link in Merel or Delta Chat, or scans the QR code, gets an encrypted chat with you. Your keys are checked along the way, so nobody can sit in between.
chat-invite-copy = Copy link
chat-invite-copied = Copied
chat-invite-mail = Send by e-mail
chat-invite-renew = Make a new link
chat-invite-renew-hint = The old link and QR code stop working.
chat-invite-close = Close
chat-invite-qr = QR code of the invitation link
chat-invite-mail-subject = Will you chat with me?
chat-invite-mail-text = I'd like to chat with you, end-to-end encrypted. Open this link in Merel or in Delta Chat (free, for phone and computer, see delta.chat):
chat-accept = Accept an invitation
chat-accept-hint = Paste an invitation link (https://i.delta.chat/#…)
chat-accept-button = Accept
chat-accept-sent = Invitation accepted. As soon as { $name } answers, the chat is ready.
chat-pending = Waiting for { $name } to answer
chat-verified = verified
chat-show-invite = Show invitation link

## Meetings

meet-today = Today
meet-week = Next 7 days
meet-plan = Plan a meeting
meet-join-link = Join with a link
meet-none-today = There are no online meetings in your calendar today.
meet-none-week = There are no online meetings in your calendar in the next 7 days.
meet-found-note = Merel finds the meeting link in your appointments and invitations.
meet-who = { $organizer } · { $count ->
    [one] 1 person
   *[other] { $count } people
}
meet-starts-in = Starts in { $minutes } min
meet-now = Now
meet-ended = Ended
meet-join = Join
meet-join-named = Join { $title }
meet-copy = Copy link
meet-copied = Copied
meet-via = Online meeting via { $service }
meet-via-link = Online meeting via a link
meet-organizer = { $name } (organiser)
meet-browser-note = Join opens the meeting in your browser. If you have the service's app, it takes over.
meet-title = Meetings
meet-intro = Here are the online meetings in your calendar, from Microsoft 365, Zoom, Google Meet and other services. Pick one to join.
meet-paste-title = Join with a link
meet-paste-hint = Paste a meeting link (https://…)
meet-paste-bad = That is not a meeting link. A meeting link starts with https:// and has no spaces.
