//! "Controleren op updates": asks GitHub which Mailcraft release is the newest and, when it is
//! newer than this build, says so in a small window with a button to the release page. Nothing
//! is downloaded or installed here; the person downloads the installer in the browser.
//!
//! The question goes out only when the person asks (Help tab) or has switched on the daily
//! check, because it shows GitHub this computer's address and that Mailcraft runs. It carries
//! no account, no identifier and no version: one `https` GET with a short timeout and a size
//! cap. The reply is data, never a link to follow: the release page is built from our own
//! repository address and a tag that must read as a version number.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::RichText;
use serde_json::Value;

use crate::helder::{self, Icon};
use crate::help::REPOSITORY;
use crate::theme::Tokens;
use crate::{MailApp, Notice};

/// Where the newest release is described.
const LATEST: &str = "https://api.github.com/repos/alainrees/mailcraft/releases/latest";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REPLY_BYTES: u64 = 512 * 1024;
/// How often the daily check asks while the app keeps running.
pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How much of the release notes the window shows.
const NOTES_CHARS: usize = 600;

/// The window's title and buttons, which tests and screen readers find them by.
pub const WINDOW: &str = "Nieuwe versie van Mailcraft";
pub const CHECK: &str = "Updates";
pub const DOWNLOAD: &str = "Downloaden";
pub const SKIP: &str = "Deze versie overslaan";
pub const LATER: &str = "Later";
pub const AUTO: &str = "Elke dag controleren op een nieuwe versie";

/// A release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// "0.2.0", without the "v".
    pub version: String,
    /// The release notes, as written on GitHub, shortened.
    pub notes: String,
}

impl Release {
    /// The release page, from our own address and the checked version; never from the reply.
    pub fn page(&self) -> String {
        format!("{REPOSITORY}/releases/tag/v{}", self.version)
    }
}

/// What one check found out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A newer release.
    Newer(Release),
    /// This build is the newest (or newer, for a build from source).
    UpToDate,
    /// No release is published yet (or the repository is not public).
    NoneYet,
    /// The question did not get an answer; the reason is for the log.
    Failed(String),
}

#[derive(Default)]
pub struct Updates {
    /// The answer of the check that is running, once it is in.
    pending: Option<Arc<Mutex<Option<Outcome>>>>,
    /// Whether the person asked for the running check (then every answer is shown).
    manual: bool,
    /// When the last check started, for the daily check.
    last: Option<Instant>,
    /// The newer release the window shows, while it is open.
    pub offer: Option<Release>,
}

impl Updates {
    pub fn running(&self) -> bool {
        self.pending.is_some()
    }
}

/// A version number "1.2.3" (a leading "v" allowed) as three numbers; anything else is `None`,
/// so a pre-release or a strange tag is never offered.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().strip_prefix('v').unwrap_or(text.trim()).split('.');
    let mut next = || parts.next().filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).and_then(|p| p.parse().ok());
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// What GitHub's reply says, compared with `current`.
pub fn judge(reply: &Value, current: &str) -> Outcome {
    let tag = reply.get("tag_name").and_then(Value::as_str).unwrap_or_default();
    let draft = reply.get("draft").and_then(Value::as_bool).unwrap_or(false);
    let prerelease = reply.get("prerelease").and_then(Value::as_bool).unwrap_or(false);
    let (Some(latest), Some(ours)) = (parse_version(tag), parse_version(current)) else {
        return Outcome::Failed(format!("unreadable version {tag:?}"));
    };
    if draft || prerelease || latest <= ours {
        return Outcome::UpToDate;
    }
    let notes = reply.get("body").and_then(Value::as_str).unwrap_or_default().trim();
    let mut short: String = notes.chars().take(NOTES_CHARS).collect();
    if notes.chars().count() > NOTES_CHARS {
        short.push('…');
    }
    Outcome::Newer(Release { version: format!("{}.{}.{}", latest.0, latest.1, latest.2), notes: short })
}

fn ask() -> Outcome {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .tls_config(crate::remote::tls())
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .https_only(true)
            .max_redirects(0)
            .user_agent("Mailcraft")
            .build(),
    );
    let mut response = match agent.get(LATEST).header("Accept", "application/vnd.github+json").call() {
        Ok(r) => r,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    if response.status().as_u16() == 404 {
        return Outcome::NoneYet;
    }
    if !response.status().is_success() {
        return Outcome::Failed(format!("HTTP {}", response.status()));
    }
    let reply = response
        .body_mut()
        .with_config()
        .limit(MAX_REPLY_BYTES)
        .read_to_vec()
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string()));
    match reply {
        Ok(v) => judge(&v, env!("CARGO_PKG_VERSION")),
        Err(e) => Outcome::Failed(e),
    }
}

impl MailApp {
    /// Starts a check on its own thread, unless one is running. `manual` when the person asked.
    pub fn check_for_updates(&mut self, ctx: &egui::Context, manual: bool) {
        if self.updates.running() {
            self.updates.manual |= manual;
            return;
        }
        let slot = Arc::new(Mutex::new(None));
        self.updates.pending = Some(slot.clone());
        self.updates.manual = manual;
        self.updates.last = Some(Instant::now());
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let outcome = ask();
            if let Ok(mut s) = slot.lock() {
                *s = Some(outcome);
            }
            ctx.request_repaint();
        });
    }

    /// Takes in what a check found. A check the person asked for always answers; the daily one
    /// speaks only of a newer release they have not chosen to skip.
    pub fn finish_update_check(&mut self, outcome: Outcome) {
        let manual = std::mem::take(&mut self.updates.manual);
        match outcome {
            Outcome::Newer(release) => {
                if manual || self.view.skipped_version.as_deref() != Some(release.version.as_str()) {
                    self.updates.offer = Some(release);
                }
            }
            Outcome::UpToDate if manual => {
                self.notice = Some(Notice::success(format!("Je hebt de nieuwste versie van Mailcraft ({}).", env!("CARGO_PKG_VERSION"))));
            }
            Outcome::NoneYet if manual => {
                self.notice = Some(Notice::info("Er staat nog geen versie van Mailcraft klaar om te downloaden."));
            }
            Outcome::Failed(reason) => {
                log::warn!("update check failed: {reason}");
                if manual {
                    self.notice = Some(Notice::warning("Controleren op updates is niet gelukt. Is er verbinding met internet?"));
                }
            }
            Outcome::UpToDate | Outcome::NoneYet => {}
        }
    }

    /// Picks up a finished check and starts the daily one when it is due; every frame.
    pub fn tick_updates(&mut self, ctx: &egui::Context) {
        let done = self.updates.pending.as_ref().and_then(|slot| slot.lock().ok().and_then(|mut s| s.take()));
        if let Some(outcome) = done {
            self.updates.pending = None;
            self.finish_update_check(outcome);
        }
        let due = self.updates.last.is_none_or(|t| t.elapsed() >= INTERVAL);
        if self.view.update_check == Some(true) && due && !self.updates.running() {
            self.check_for_updates(ctx, false);
        }
    }
}

/// The window that offers a newer release.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(release) = app.updates.offer.clone() else { return };
    let mut close = false;
    let closed = helder::dialog(ctx, t, WINDOW, 460.0, |ui| {
        helder::card_frame(t).inner_margin(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                helder::badge(ui, t, Icon::Download);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("Mailcraft {} is beschikbaar.", release.version)).size(18.0).strong().color(t.text));
                    ui.label(RichText::new(format!("Je gebruikt nu versie {}.", env!("CARGO_PKG_VERSION"))).size(14.0).color(t.text_dim));
                });
            });
            if !release.notes.is_empty() {
                ui.add_space(12.0);
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| helder::paragraph(ui, t, &release.notes));
            }
            ui.add_space(12.0);
            ui.add(
                egui::Label::new(
                    RichText::new("Downloaden opent de pagina met de installatiebestanden in je browser. Installeer daarna over deze versie heen; je accounts en mail blijven bewaard.")
                        .size(12.0)
                        .color(t.text_dim),
                )
                .wrap(),
            );
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            helder::quiet(ui, t, |ui| {
                if helder::text_button(ui, t, SKIP).on_hover_text("Niet meer over deze versie beginnen; wel over een nieuwere").clicked() {
                    app.view.skipped_version = Some(release.version.clone());
                    app.save_view(ctx);
                    close = true;
                }
            });
            helder::right(ui, |ui| {
                if helder::primary_button(ui, t, Icon::Download, DOWNLOAD).clicked() {
                    if let Err(e) = open::that_detached(release.page()) {
                        log::warn!("could not open the release page: {e}");
                        app.notice = Some(Notice::error(format!("Openen is niet gelukt: {}", release.page())));
                    }
                    close = true;
                }
                helder::secondary(ui, t, |ui| {
                    if ui.button(RichText::new(LATER).size(14.0).color(t.text)).clicked() {
                        close = true;
                    }
                });
            });
        });
    });
    close |= closed;
    if close {
        app.updates.offer = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_plain_version_numbers_count() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.10.3"), Some((1, 10, 3)));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("v1.2.3-beta"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        assert_eq!(parse_version("../evil"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn a_newer_release_is_offered_with_its_page_built_from_our_own_address() {
        let reply = json!({"tag_name": "v0.2.0", "body": "Nieuw: zoeken.", "html_url": "https://evil.example/", "draft": false, "prerelease": false});
        let Outcome::Newer(r) = judge(&reply, "0.1.0") else { panic!("expected a newer release") };
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.notes, "Nieuw: zoeken.");
        assert_eq!(r.page(), "https://github.com/alainrees/mailcraft/releases/tag/v0.2.0");
        assert_eq!(judge(&reply, "0.2.0"), Outcome::UpToDate);
        assert_eq!(judge(&reply, "0.3.0"), Outcome::UpToDate);
        assert_eq!(judge(&json!({"tag_name": "v9.0.0", "prerelease": true}), "0.1.0"), Outcome::UpToDate);
        assert!(matches!(judge(&json!({"tag_name": "nightly"}), "0.1.0"), Outcome::Failed(_)));
    }

    #[test]
    fn long_notes_are_cut() {
        let reply = json!({"tag_name": "v1.0.0", "body": "x".repeat(2000)});
        let Outcome::Newer(r) = judge(&reply, "0.1.0") else { panic!() };
        assert_eq!(r.notes.chars().count(), NOTES_CHARS + 1);
    }
}
