# Merel for Android

A native Jetpack Compose app on top of Merel's Rust core (`crates/mobile`, through UniFFI). The
screens follow the approved designs in the project's `ontwerp/mobiel/` folder.

What works now: sample mail, adding an IMAP/SMTP account (password sealed with a key in the
Android Keystore), the folder tree, the message list with search, filters, pull to refresh and
swipe to archive or delete, and reading a message. Server certificates are checked by Android's
own trust store (rustls-platform-verifier). The calendar, tasks and contacts work as on the iPhone. Microsoft 365 and Gmail sign in in a Custom Tab, back to a listener on `127.0.0.1` as on the computer. Chat works as on the iPhone, always end-to-end encrypted.

## Building

1. Install Android Studio (it brings the SDK; add the NDK under SDK Manager > SDK Tools), and once:
   `rustup target add aarch64-linux-android x86_64-linux-android` and `cargo install cargo-ndk`.
2. `ANDROID_NDK_HOME=<path to the NDK> apps/android/scripts/build-core.sh`
3. Open `apps/android` in Android Studio and press Run, or `./gradlew assembleDebug` for
   `app/build/outputs/apk/debug/app-debug.apk`.

CI (`.github/workflows/mobile.yml`) builds the APK on every pull request; it is attached to the
run as `merel-android-apk`. To install it on a phone, allow installing from that source when
Android asks.
