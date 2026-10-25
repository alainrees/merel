# Merel for the iPhone

A native SwiftUI app on top of Merel's Rust core (`crates/mobile`, through UniFFI). The
screens follow the approved designs in the project's `ontwerp/mobiel/` folder.

What works now: sample mail, adding an IMAP/SMTP account (password in the iOS keychain), the
folder tree with "Alle accounts" on top, the message list with pull to refresh and swipe actions
(archive, delete, flag, read/unread), reading a message, and the calendar. Search works as on the
computer (the search index, search words such as `van:`, the text of attachments), in all folders or
this one; search on meaning, switched on under Mappen > Zoeken, downloads the small language model
once. Tasks: the open ones under Te laat, Vandaag, Morgen and Later, a tap on the circle to finish one,
and a form for the due date, priority, repeat (tasks on the phone) and where to keep a new one. Contacts:
the list with search, a contact's card with buttons to mail and call, and a form for the name,
company, job title, email addresses, phone numbers and notes. Microsoft 365 and Gmail sign in in Safari inside the app, as on the computer: the provider sends
the person back to a listener on `127.0.0.1`, so Merel never sees the password. Chat follows.

## Running it on your own iPhone (free Apple ID)

1. Install Xcode from the App Store, and once: `brew install xcodegen` and
   `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`.
2. In the repository: `apps/ios/scripts/build-core.sh` (builds the core, a few minutes).
3. `cd apps/ios && xcodegen && open Merel.xcodeproj`.
4. Once, so Xcode signs with your Apple ID ("Personal Team") and keeps it after `xcodegen`:
   `apps/ios/scripts/signing.sh <team-id> <bundle-id>`, for example
   `apps/ios/scripts/signing.sh 95SUS87Z39 nl.yourname.merel`, then `xcodegen` again. The team id
   is under Signing & Capabilities once you picked your team there; the bundle identifier must be
   one nobody else has taken. If Xcode says "0 valid identities", import Apple's intermediate
   certificate: `curl -O https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer && security import AppleWWDRCAG3.cer -k ~/Library/Keychains/login.keychain-db`.
5. Connect the iPhone, pick it as the run destination and press Run. On the iPhone, allow the
   developer once under Settings > General > VPN & Device Management, and turn on Developer
   Mode when iOS asks.

After pulling new code: `apps/ios/scripts/build-core.sh`, then `xcodegen` (it picks up new
files), then Run. Xcode rewrites `Merel/Localizable.xcstrings` while it builds; if `git pull`
refuses because of that file, drop the change first with
`git checkout -- apps/ios/Merel/Localizable.xcstrings`.

With a free Apple ID the app runs for 7 days; then press Run again. The App Store and
TestFlight need the paid Apple Developer Program.

CI (`.github/workflows/mobile.yml`) builds the app for the simulator on every pull request.
