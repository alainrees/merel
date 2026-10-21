//! "Controleren op updates": asks GitHub which Merel release is the newest and, when it is
//! newer than this build, says so in a small window. "Bijwerken" there downloads, checks and
//! installs it and starts Merel again ([`crate::install`]); where Merel cannot replace itself
//! (run from source, from a `.deb`) the button opens the release page instead.
//!
//! The question goes out only when the person asks (Help tab) or has switched on the daily
//! check, because it shows GitHub this computer's address and that Merel runs. It carries
//! no account, no identifier and no version: one `https` GET with a short timeout and a size
//! cap. The reply is data, never a link to follow: the release page is built from our own
//! repository address and a tag that must read as a version number.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::RichText;
use serde_json::Value;

use crate::helder::{self, Icon};
use crate::help::REPOSITORY;
use crate::install::{self, Job, Restart, Stage};
use crate::theme::Tokens;
use crate::{MailApp, Notice};

/// Where the newest release is described.
const LATEST: &str = "https://api.github.com/repos/alainrees/merel/releases/latest";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REPLY_BYTES: u64 = 512 * 1024;
/// How often the daily check asks while the app keeps running.
pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Where the list GitHub adds to the release notes begins.
const GENERATED: [&str; 3] = ["## What's Changed", "## New Contributors", "**Full Changelog**"];

/// How much of the release notes the window shows.
const NOTES_CHARS: usize = 2000;

/// The window's title and buttons, which tests and screen readers find them by.
pub const WINDOW: &str = "Nieuwe versie van Merel";
pub const CHECK: &str = "Updates";
pub const DOWNLOAD: &str = "Downloaden";
pub const UPDATE: &str = "Bijwerken";
pub const RESTART: &str = "Opnieuw starten";
pub const CANCEL: &str = "Annuleren";
pub const PAGE: &str = "Downloadpagina openen";
pub const SKIP: &str = "Deze versie overslaan";
pub const LATER: &str = "Later";
pub const AUTO: &str = "Elke dag controleren op een nieuwe versie";

/// A release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// "0.2.0", without the "v".
    pub version: String,
    /// Our part of the release notes on GitHub, Markdown, shortened.
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
    /// The update that is downloading or installing, once the person chose "Bijwerken".
    pub job: Option<Job>,
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
    let body = reply.get("body").and_then(Value::as_str).unwrap_or_default();
    // Our own notes come first; GitHub's list of pull requests after them is for developers.
    let notes = GENERATED.iter().filter_map(|mark| body.find(mark)).min().map_or(body, |end| &body[..end]).trim();
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
            .user_agent("Merel")
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
                self.notice = Some(Notice::success(format!("Je hebt de nieuwste versie van Merel ({}).", env!("CARGO_PKG_VERSION"))));
            }
            Outcome::NoneYet if manual => {
                self.notice = Some(Notice::info("Er staat nog geen versie van Merel klaar om te downloaden."));
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

/// Opens a release's page in the browser, saying so when that does not work.
fn open_page(app: &mut MailApp, release: &Release) {
    if let Err(e) = open::that_detached(release.page()) {
        log::warn!("could not open the release page: {e}");
        app.notice = Some(Notice::error(format!("Openen is niet gelukt: {}", release.page())));
    }
}

/// The window that offers a newer release, and then walks through the update.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    if app.updates.job.is_some() {
        return progress(app, ctx, t);
    }
    let Some(release) = app.updates.offer.clone() else { return };
    let here = install::Install::detect().filter(|_| install::SIGNING_KEY.is_some());
    let mut close = false;
    let closed = helder::dialog(ctx, t, WINDOW, 460.0, |ui| {
        if here.is_some() {
            helder::step_dots(ui, t, 0, STEPS);
            ui.add_space(12.0);
        }
        helder::card_frame(t).inner_margin(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                helder::badge(ui, t, Icon::Download);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("Merel {} is beschikbaar.", release.version)).size(18.0).strong().color(t.text));
                    ui.label(RichText::new(format!("Je gebruikt nu versie {}.", env!("CARGO_PKG_VERSION"))).size(14.0).color(t.text_dim));
                });
            });
            if !release.notes.is_empty() {
                ui.add_space(12.0);
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| crate::release_notes::show(ui, t, &release.notes));
            }
            ui.add_space(12.0);
            let how = if here.is_some() {
                "Bijwerken downloadt de nieuwe versie, controleert de handtekening, installeert hem en start Merel opnieuw. Je accounts en mail blijven bewaard."
            } else {
                "Downloaden opent de pagina met de installatiebestanden in je browser. Installeer daarna over deze versie heen; je accounts en mail blijven bewaard."
            };
            ui.add(egui::Label::new(RichText::new(how).size(12.0).color(t.text_dim)).wrap());
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
                match (&here, install::SIGNING_KEY) {
                    (Some(place), Some(key)) => {
                        if helder::primary_button(ui, t, Icon::Download, UPDATE).clicked() {
                            app.updates.job = Some(Job::start(ctx, place.clone(), &release.version, key));
                        }
                    }
                    _ => {
                        if helder::primary_button(ui, t, Icon::Download, DOWNLOAD).clicked() {
                            open_page(app, &release);
                            close = true;
                        }
                    }
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

/// The steps the wizard shows as dots: offer, download, check, install.
const STEPS: usize = 4;

/// The update under way: a progress bar while it downloads, then the check, the install and
/// the button that starts the new version; or what went wrong, with the release page.
fn progress(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(job) = &app.updates.job else { return };
    let stage = job.stage();
    let version = job.version.clone();
    let release = Release { version: version.clone(), notes: String::new() };
    let (step, icon, title, text) = match &stage {
        Stage::Downloading { .. } => (1, Icon::Download, format!("Merel {version} downloaden"), String::new()),
        Stage::Checking => (
            2,
            Icon::ShieldCheck,
            "Handtekening controleren".to_owned(),
            "Merel controleert of de download van Merel zelf komt en onderweg niet is veranderd.".to_owned(),
        ),
        Stage::Installing => (3, Icon::RefreshCw, format!("Merel {version} installeren"), "Even geduld, dit duurt een paar seconden.".to_owned()),
        Stage::Ready(Restart::Installer(_)) => (
            3,
            Icon::CircleCheck,
            format!("Merel {version} staat klaar"),
            "Merel sluit af, installeert de nieuwe versie en start daarna vanzelf weer.".to_owned(),
        ),
        Stage::Ready(Restart::Relaunch(_)) => (
            3,
            Icon::CircleCheck,
            format!("Merel {version} is geïnstalleerd"),
            "Start Merel opnieuw om de nieuwe versie te gebruiken. Doe je dat later, dan krijg je hem bij de volgende start.".to_owned(),
        ),
        Stage::Failed(reason) => (
            0,
            Icon::CircleAlert,
            "Bijwerken is niet gelukt".to_owned(),
            format!("{reason} Je kunt de nieuwe versie ook zelf downloaden en installeren."),
        ),
    };
    let mut close = false;
    let closed = helder::dialog(ctx, t, WINDOW, 460.0, |ui| {
        if !matches!(stage, Stage::Failed(_)) {
            helder::step_dots(ui, t, step, STEPS);
            ui.add_space(12.0);
        }
        helder::card_frame(t).inner_margin(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                helder::badge(ui, t, icon);
                ui.add_space(8.0);
                ui.label(RichText::new(&title).size(18.0).strong().color(t.text));
            });
            ui.add_space(12.0);
            if let Stage::Downloading { done, total } = stage {
                let fraction = total.filter(|t| *t > 0).map(|t| done as f32 / t as f32);
                helder::progress_bar(ui, t, fraction);
                ui.add_space(6.0);
                let mb = |b: u64| b as f64 / 1_000_000.0;
                let line = match total {
                    Some(total) => format!("{:.0} van {:.0} MB", mb(done), mb(total)),
                    None => format!("{:.0} MB", mb(done)),
                };
                ui.label(RichText::new(line).size(12.0).color(t.text_dim));
            } else {
                helder::paragraph(ui, t, &text);
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            helder::right(ui, |ui| match &stage {
                Stage::Downloading { .. } => {
                    helder::secondary(ui, t, |ui| {
                        if ui.button(RichText::new(CANCEL).size(14.0).color(t.text)).clicked() {
                            close = true;
                        }
                    });
                }
                Stage::Checking | Stage::Installing => {}
                Stage::Ready(restart) => {
                    if helder::primary_button(ui, t, Icon::RefreshCw, RESTART).clicked() {
                        let handover = install::hand_over(&app.engine);
                        match restart.start(handover.as_ref()) {
                            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                            Err(e) => {
                                log::warn!("could not start the new version: {e}");
                                if let Some(key) = &handover {
                                    install::take_back(&app.engine, key);
                                }
                                app.notice = Some(Notice::error("Opnieuw starten is niet gelukt. Start Merel zelf opnieuw."));
                            }
                        }
                        close = true;
                    }
                    helder::secondary(ui, t, |ui| {
                        if ui.button(RichText::new(LATER).size(14.0).color(t.text)).clicked() {
                            close = true;
                        }
                    });
                }
                Stage::Failed(_) => {
                    if helder::primary_button(ui, t, Icon::Globe, PAGE).clicked() {
                        open_page(app, &release);
                        close = true;
                    }
                    helder::secondary(ui, t, |ui| {
                        if ui.button(RichText::new(helder::CLOSE).size(14.0).color(t.text)).clicked() {
                            close = true;
                        }
                    });
                }
            });
        });
    });
    // While it installs the window stays: half an install must not be walked away from.
    let busy = matches!(stage, Stage::Checking | Stage::Installing);
    if (close || closed) && !busy {
        if let Some(job) = app.updates.job.take() {
            job.cancel();
        }
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
        assert_eq!(r.page(), "https://github.com/alainrees/merel/releases/tag/v0.2.0");
        assert_eq!(judge(&reply, "0.2.0"), Outcome::UpToDate);
        assert_eq!(judge(&reply, "0.3.0"), Outcome::UpToDate);
        assert_eq!(judge(&json!({"tag_name": "v9.0.0", "prerelease": true}), "0.1.0"), Outcome::UpToDate);
        assert!(matches!(judge(&json!({"tag_name": "nightly"}), "0.1.0"), Outcome::Failed(_)));
    }

    #[test]
    fn githubs_own_list_is_left_out() {
        let body = "## Nieuw in Merel 0.1.6\n\n- Mappen als boom.\n\n## What's Changed\n* Folder tree by @someone in https://github.com/x/y/pull/1\n\n**Full Changelog**: https://x";
        let Outcome::Newer(r) = judge(&json!({"tag_name": "v0.1.6", "body": body}), "0.1.5") else { panic!() };
        assert_eq!(r.notes, "## Nieuw in Merel 0.1.6\n\n- Mappen als boom.");
    }

    #[test]
    fn long_notes_are_cut() {
        let reply = json!({"tag_name": "v1.0.0", "body": "x".repeat(NOTES_CHARS + 500)});
        let Outcome::Newer(r) = judge(&reply, "0.1.0") else { panic!() };
        assert_eq!(r.notes.chars().count(), NOTES_CHARS + 1);
    }
}
