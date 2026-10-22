//! The unread count on Merel's app icon and the computer's own notifications, as Outlook has
//! them: a red count on the Dock icon (macOS) or the launcher (Linux, where the desktop shows
//! one), and a notification in the corner of the screen for new mail and reminders. A click on
//! a notification is handed back to the app by the id it was shown with.
//!
//! macOS: the Dock tile's badge and the UserNotifications framework, which only works inside
//! the Merel.app bundle (not for `cargo run`, where notifications are left out). Linux:
//! notifications and the launcher count over D-Bus. Windows: toasts, under the id the
//! installer gives Merel's Start menu shortcut; the taskbar has no count, the window title
//! carries it ("Merel (3)").

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

/// The app's id, as in the macOS bundle and on the Windows Start menu shortcut.
pub const APP_ID: &str = "io.github.alainrees.merel";

/// What a notification is about, for its sound and how the desktop files it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mail,
    Reminder,
}

/// One notification. `id` comes back from [`take_clicked`] when the person clicks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub id: String,
    pub title: String,
    pub body: String,
    pub kind: Kind,
    pub sound: bool,
}

static CLICKED: Mutex<Vec<String>> = Mutex::new(Vec::new());
type Waker = Box<dyn Fn() + Send + Sync>;
static WAKER: OnceLock<Waker> = OnceLock::new();

/// Called from any thread when a notification is clicked.
fn clicked(id: String) {
    if let Ok(mut c) = CLICKED.lock() {
        c.push(id);
    }
    if let Some(wake) = WAKER.get() {
        wake();
    }
}

/// Whether the computer shows the count on the app icon: unknown until it says (see
/// [`badge_blocked`]).
static BADGE_SETTING: AtomicU8 = AtomicU8::new(BADGE_UNKNOWN);
const BADGE_UNKNOWN: u8 = 0;
const BADGE_ALLOWED: u8 = 1;
const BADGE_BLOCKED: u8 = 2;
/// Set when the computer's answer may have changed what the badge shows, so it is sent again.
static BADGE_RESEND: AtomicBool = AtomicBool::new(false);

/// Called from any thread with what the computer allows for the app icon's count.
#[allow(dead_code)]
fn badge_allowed(allowed: bool) {
    BADGE_SETTING.store(if allowed { BADGE_ALLOWED } else { BADGE_BLOCKED }, Ordering::Relaxed);
    BADGE_RESEND.store(true, Ordering::Relaxed);
    if let Some(wake) = WAKER.get() {
        wake();
    }
}

/// Whether the computer's own settings keep the count off the app icon (macOS: Systeeminstellingen
/// > Berichtgeving > Merel > "Kenteken op appsymbool"), so the app can say where to switch it on.
pub fn badge_blocked() -> bool {
    BADGE_SETTING.load(Ordering::Relaxed) == BADGE_BLOCKED
}

/// The ids of the notifications clicked since the last call, oldest first.
pub fn take_clicked() -> Vec<String> {
    CLICKED.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default()
}

/// What to call when a notification is clicked, so the app draws again and picks it up. Only
/// the first waker counts.
pub fn set_waker(wake: impl Fn() + Send + Sync + 'static) {
    let _ = WAKER.set(Box::new(wake));
}

/// Shows the count and the notifications. Make one, on the main thread, and keep it.
pub struct Notifier {
    badge: Option<u32>,
    prepared: bool,
    inner: platform::Inner,
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier {
    pub fn new() -> Self {
        Notifier { badge: None, prepared: false, inner: platform::Inner::new() }
    }

    /// Asks the computer, once, whether Merel may show notifications (macOS asks the person
    /// the first time). Call it when notifications are switched on, before the first one.
    pub fn prepare(&mut self) {
        if !self.prepared {
            self.prepared = true;
            self.inner.prepare();
        }
    }

    /// The count on the app icon; 0 takes it away. Only a change reaches the computer, or the
    /// same count again once the computer's settings for it have been read. On macOS the Dock
    /// only shows the count of an app that may badge, so this asks for that first.
    pub fn set_badge(&mut self, count: u32) {
        self.prepare();
        if self.badge != Some(count) || BADGE_RESEND.swap(false, Ordering::Relaxed) {
            self.badge = Some(count);
            self.inner.set_badge(count);
        }
    }

    pub fn show(&mut self, alert: &Alert) {
        self.prepare();
        self.inner.show(alert);
    }

    /// Reads the computer's notification settings again, as the person may have changed them
    /// in the meantime; [`badge_blocked`] tells the answer.
    pub fn check_settings(&mut self) {
        if self.prepared {
            self.inner.check_settings();
        }
    }
}

/// The text on the badge: nothing for 0, and "999+" past that, as the Dock has little room.
pub fn badge_label(count: u32) -> Option<String> {
    match count {
        0 => None,
        1..=999 => Some(count.to_string()),
        _ => Some("999+".to_string()),
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, MainThreadMarker, define_class, msg_send};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNNotificationResponse, UNNotificationSetting,
        UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    use super::Alert;

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the class keeps no state.
        #[unsafe(super(NSObject))]
        #[name = "MerelNotificationDelegate"]
        struct Delegate;

        // SAFETY: NSObjectProtocol has no extra requirements.
        unsafe impl NSObjectProtocol for Delegate {}

        // SAFETY: the method below has the signature the protocol declares.
        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
                super::clicked(response.notification().request().identifier().to_string());
                done.call(());
            }
        }
    );

    impl Delegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            // SAFETY: NSObject's init on a freshly allocated object.
            unsafe { msg_send![super(this), init] }
        }
    }

    pub struct Inner {
        /// The notification center's delegate; the center holds it weakly, so it lives here.
        delegate: Option<Retained<Delegate>>,
    }

    /// Asks for the notification settings; the answer comes on another thread.
    fn read_settings() {
        let Some(center) = center() else { return };
        let answer = block2::RcBlock::new(|settings: std::ptr::NonNull<UNNotificationSettings>| {
            // SAFETY: the framework hands a valid settings object for the call's duration.
            let settings = unsafe { settings.as_ref() };
            let badge = settings.badgeSetting();
            log::info!("badge setting: {}", badge.0);
            super::badge_allowed(badge != UNNotificationSetting::Disabled);
        });
        center.getNotificationSettingsWithCompletionHandler(&answer);
    }

    /// The notification center, only inside an app bundle: outside one the framework aborts.
    fn center() -> Option<Retained<UNUserNotificationCenter>> {
        NSBundle::mainBundle().bundleIdentifier()?;
        Some(UNUserNotificationCenter::currentNotificationCenter())
    }

    impl Inner {
        pub fn new() -> Self {
            Inner { delegate: None }
        }

        pub fn prepare(&mut self) {
            let Some(center) = center() else {
                log::info!("not inside Merel.app: no notifications");
                return;
            };
            let delegate = Delegate::new();
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            self.delegate = Some(delegate);
            // The Badge option too: without it the Dock leaves the count off Merel's icon.
            // Someone who answered before Badge was asked for is not asked again; they switch
            // it on in System Settings, which the app points to (see `badge_blocked`).
            let answer = block2::RcBlock::new(|granted: objc2::runtime::Bool, _error: *mut NSError| {
                log::info!("notifications allowed: {}", granted.as_bool());
                read_settings();
            });
            let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge;
            center.requestAuthorizationWithOptions_completionHandler(options, &answer);
        }

        pub fn check_settings(&mut self) {
            read_settings();
        }

        pub fn set_badge(&mut self, count: u32) {
            let Some(mtm) = MainThreadMarker::new() else { return };
            let label = super::badge_label(count).map(|l| NSString::from_str(&l));
            NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
        }

        pub fn show(&mut self, alert: &Alert) {
            let Some(center) = center() else { return };
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&alert.title));
            content.setBody(&NSString::from_str(&alert.body));
            if alert.sound {
                content.setSound(Some(&UNNotificationSound::defaultSound()));
            }
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&alert.id), &content, None);
            center.addNotificationRequest_withCompletionHandler(&request, None);
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::collections::HashMap;
    use std::sync::mpsc::{Sender, channel};

    use notify_rust::{Hint, Notification};
    use zbus::zvariant::Value;

    use super::{Alert, Kind};

    enum Job {
        Badge(u32),
        Show(Alert),
    }

    /// D-Bus calls wait for an answer, so they run on a thread of their own.
    pub struct Inner {
        jobs: Option<Sender<Job>>,
    }

    impl Inner {
        pub fn new() -> Self {
            let (tx, rx) = channel::<Job>();
            let started = std::thread::Builder::new().name("merel-notify".into()).spawn(move || {
                let mut bus: Option<zbus::blocking::Connection> = None;
                for job in rx {
                    match job {
                        Job::Badge(count) => {
                            if bus.is_none() {
                                bus = zbus::blocking::Connection::session().map_err(|e| log::info!("no session bus: {e}")).ok();
                            }
                            if let Some(bus) = &bus {
                                badge(bus, count);
                            }
                        }
                        Job::Show(alert) => show(alert),
                    }
                }
            });
            Inner { jobs: started.ok().map(|_| tx) }
        }

        pub fn prepare(&mut self) {}

        pub fn check_settings(&mut self) {}

        pub fn set_badge(&mut self, count: u32) {
            if let Some(jobs) = &self.jobs {
                let _ = jobs.send(Job::Badge(count));
            }
        }

        pub fn show(&mut self, alert: &Alert) {
            if let Some(jobs) = &self.jobs {
                let _ = jobs.send(Job::Show(alert.clone()));
            }
        }
    }

    /// The Unity launcher API, which KDE's task manager, Dash to Dock and Plank read too.
    fn badge(bus: &zbus::blocking::Connection, count: u32) {
        let mut props: HashMap<&str, Value> = HashMap::new();
        props.insert("count", Value::from(i64::from(count)));
        props.insert("count-visible", Value::from(count > 0));
        let body = ("application://merel.desktop", props);
        if let Err(e) = bus.emit_signal(None::<&str>, "/io/github/alainrees/merel", "com.canonical.Unity.LauncherEntry", "Update", &body) {
            log::info!("launcher count not sent: {e}");
        }
    }

    fn show(alert: Alert) {
        let mut n = Notification::new();
        n.appname("Merel").icon("merel").summary(&alert.title).body(&alert.body).action("default", "Openen");
        n.hint(Hint::DesktopEntry("merel".into()));
        n.hint(Hint::Category(if alert.kind == Kind::Mail { "email.arrived" } else { "im" }.into()));
        if alert.sound {
            n.sound_name(if alert.kind == Kind::Mail { "message-new-email" } else { "alarm-clock-elapsed" });
        } else {
            n.hint(Hint::SuppressSound(true));
        }
        match n.show() {
            // Waiting for the click holds a thread until the notification goes away.
            Ok(handle) => {
                let _ = std::thread::Builder::new().name("merel-notify-click".into()).spawn(move || {
                    handle.wait_for_action(|action| {
                        if action == "default" {
                            super::clicked(alert.id);
                        }
                    });
                });
            }
            Err(e) => log::info!("notification not shown: {e}"),
        }
    }
}

#[cfg(windows)]
mod platform {
    use notify_rust::{Notification, NotificationResponse};

    use super::{APP_ID, Alert, Kind};

    pub struct Inner;

    impl Inner {
        pub fn new() -> Self {
            Inner
        }

        pub fn prepare(&mut self) {}

        pub fn check_settings(&mut self) {}

        /// The taskbar shows no count of its own; the window title carries it.
        pub fn set_badge(&mut self, _count: u32) {}

        pub fn show(&mut self, alert: &Alert) {
            let alert = alert.clone();
            let _ = std::thread::Builder::new().name("merel-notify".into()).spawn(move || {
                let mut n = Notification::new();
                n.app_id(APP_ID).summary(&alert.title).body(&alert.body);
                if alert.sound {
                    n.sound_name(if alert.kind == Kind::Mail { "Mail" } else { "Reminder" });
                }
                match n.show() {
                    Ok(handle) => {
                        let _ = handle.wait_for_response(|response: &NotificationResponse| {
                            if matches!(response, NotificationResponse::Default | NotificationResponse::Action(_)) {
                                super::clicked(alert.id.clone());
                            }
                        });
                    }
                    Err(e) => log::info!("notification not shown: {e}"),
                }
            });
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::Alert;

    pub struct Inner;

    impl Inner {
        pub fn new() -> Self {
            Inner
        }
        pub fn prepare(&mut self) {}
        pub fn check_settings(&mut self) {}
        pub fn set_badge(&mut self, _count: u32) {}
        pub fn show(&mut self, _alert: &Alert) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_hides_at_zero_and_stays_short() {
        assert_eq!(badge_label(0), None);
        assert_eq!(badge_label(7).as_deref(), Some("7"));
        assert_eq!(badge_label(999).as_deref(), Some("999"));
        assert_eq!(badge_label(1500).as_deref(), Some("999+"));
    }

    #[test]
    fn clicks_are_handed_over_once() {
        clicked("mail:1".into());
        clicked("appt:2".into());
        assert_eq!(take_clicked(), vec!["mail:1".to_string(), "appt:2".to_string()]);
        assert!(take_clicked().is_empty());
    }
}
