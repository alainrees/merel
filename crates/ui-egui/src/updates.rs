//! "Controleren op updates": asks GitHub which Merel release is the newest and, when it is
//! newer than this build, says so in a small window. "Bijwerken" there downloads, checks and
//! installs it and starts Merel again ([`crate::install`]); where Merel cannot replace itself
//! (run from source, from a `.deb`) the button opens the release page instead.
//!
//! An installed Merel, one that can update itself, asks at every start, and a newer release
//! must be installed before Merel can be used ([`required_screen`]): a fix for a vulnerability
//! then reaches everyone. Without internet Merel starts as usual. While it keeps running it
//! asks again every hour; a release found then does not take the window away from work
//! but puts a red "!" on the gear at the bottom left ([`badge`]), and is required at the next
//! start. A build from source asks
//! only when the person asks (Help tab) or has switched on the daily check, and never blocks.
//! The question shows GitHub this computer's address and that Merel runs. It carries
//! no account, no identifier and no version: one `https` GET with a short timeout and a size
//! cap. The reply is data, never a link to follow: the release page is built from our own
//! repository address and a tag that must read as a version number.

use crate::i18n::tr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

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
/// How often an installed Merel asks while it keeps running, so the gear soon shows a release.
pub const INSTALLED_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Where the list GitHub adds to the release notes begins.
const GENERATED: [&str; 3] = ["## What's Changed", "## New Contributors", "**Full Changelog**"];

/// How much of the release notes the window shows.
const NOTES_CHARS: usize = 2000;

/// The window's title and buttons, which tests and screen readers find them by.
pub fn window_label() -> String {
    tr("updates-title")
}
pub fn check() -> String {
    tr("updates-check")
}
pub fn download() -> String {
    tr("updates-download")
}
pub fn update() -> String {
    tr("updates-update")
}
pub fn restart_label() -> String {
    tr("updates-restart")
}
pub fn cancel() -> String {
    tr("cancel")
}
pub fn page_label() -> String {
    tr("updates-page")
}
pub fn skip() -> String {
    tr("updates-skip")
}
pub fn later() -> String {
    tr("updates-later")
}
pub fn auto() -> String {
    tr("updates-auto")
}
pub fn retry() -> String {
    tr("updates-retry")
}

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
    /// When the last check started, for the next one. Wall-clock time, so the hours a laptop
    /// sleeps count too (a monotonic clock stands still then on macOS).
    last: Option<SystemTime>,
    /// The newer release the window shows, while it is open.
    pub offer: Option<Release>,
    /// The update that is downloading or installing, once the person chose "Bijwerken".
    pub job: Option<Job>,
    /// Whether this Merel can install a new version itself (an installed app, not a build
    /// from source): then it asks at every start, and a newer release is required.
    pub self_update: bool,
    /// The offer is a required update: Merel cannot be used until it is installed.
    pub required: bool,
    /// The newest release a check found, while it is newer than this build: the gear shows it
    /// with a red "!" and Bestand > Updates offers it, also after "Later".
    pub newest: Option<Release>,
    /// Whether a check has answered since Merel started. Only the first one, at start, makes an
    /// update required; one found later waits on the gear until the next start.
    answered: bool,
}

impl Updates {
    pub fn new() -> Self {
        Updates { self_update: install::available(), ..Default::default() }
    }

    pub fn running(&self) -> bool {
        self.pending.is_some()
    }
}

/// The release the gear points at with a red "!", if any: a newer one that is not required
/// yet (a required one fills the window) and not skipped by the person.
pub fn badge(app: &MailApp) -> Option<&Release> {
    let newest = app.updates.newest.as_ref()?;
    let skipped = !app.updates.self_update && app.view.skipped_version.as_deref() == Some(newest.version.as_str());
    (!app.updates.required && !skipped).then_some(newest)
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
        self.updates.last = Some(SystemTime::now());
        // Tests give the answer themselves, through `finish_update_check`, and stay offline.
        if cfg!(test) {
            return;
        }
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
        let first = !std::mem::replace(&mut self.updates.answered, true);
        if let Outcome::UpToDate | Outcome::NoneYet = outcome {
            self.updates.newest = None;
        }
        match outcome {
            Outcome::Newer(release) if self.updates.self_update && first => {
                self.updates.newest = Some(release.clone());
                self.updates.offer = Some(release);
                self.updates.required = true;
            }
            // Found while Merel is in use: the gear shows it; the window only when asked.
            Outcome::Newer(release) if self.updates.self_update => {
                self.updates.newest = Some(release.clone());
                if manual {
                    self.updates.offer = Some(release);
                }
            }
            Outcome::Newer(release) => {
                self.updates.newest = Some(release.clone());
                if manual || self.view.skipped_version.as_deref() != Some(release.version.as_str()) {
                    self.updates.offer = Some(release);
                }
            }
            Outcome::UpToDate if manual => {
                self.notice = Some(Notice::success(crate::i18n::tr_args("updates-up-to-date", &[("version", env!("CARGO_PKG_VERSION").into())])));
            }
            Outcome::NoneYet if manual => {
                self.notice = Some(Notice::info(tr("updates-none-yet")));
            }
            Outcome::Failed(reason) => {
                log::warn!("update check failed: {reason}");
                if manual {
                    self.notice = Some(Notice::warning(tr("updates-check-failed")));
                }
            }
            Outcome::UpToDate | Outcome::NoneYet => {}
        }
    }

    /// Picks up a finished check and starts the next when it is due; every frame. An installed
    /// Merel asks at every start and every hour after, whatever the daily setting says.
    pub fn tick_updates(&mut self, ctx: &egui::Context) {
        let done = self.updates.pending.as_ref().and_then(|slot| slot.lock().ok().and_then(|mut s| s.take()));
        if let Some(outcome) = done {
            self.updates.pending = None;
            self.finish_update_check(outcome);
        }
        let every = if self.updates.self_update { INSTALLED_INTERVAL } else { INTERVAL };
        // A clock set back makes the check due, rather than never.
        let due = self.updates.last.is_none_or(|t| !t.elapsed().is_ok_and(|e| e < every));
        if (self.updates.self_update || self.view.update_check == Some(true)) && due && !self.updates.running() {
            self.check_for_updates(ctx, false);
        }
    }
}

/// Opens a release's page in the browser, saying so when that does not work.
fn open_page(app: &mut MailApp, release: &Release) {
    if let Err(e) = open::that_detached(release.page()) {
        log::warn!("could not open the release page: {e}");
        app.notice = Some(Notice::error(crate::i18n::tr_args("updates-open-failed", &[("page", release.page().into())])));
    }
}

/// The window that offers a newer release, and then walks through the update. A required
/// update is not a window but [`required_screen`].
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    if app.updates.required {
        return;
    }
    if app.updates.job.is_some() {
        let mut close = false;
        let closed = helder::dialog(ctx, t, &window_label(), 460.0, |ui| close = progress(app, ui, t));
        return end_progress(app, close || closed);
    }
    let Some(release) = app.updates.offer.clone() else { return };
    let mut close = false;
    let closed = helder::dialog(ctx, t, &window_label(), 460.0, |ui| close = offer(app, ui, t, &release));
    if close || closed {
        app.updates.offer = None;
    }
}

/// Bestand > Updates, where the gear leads: the version in use, a newer one with "Bijwerken"
/// when there is one, a button to check now and the daily check.
pub fn page(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.label(RichText::new(check()).size(helder::TITLE_TEXT).strong().color(t.text));
    ui.add_space(4.0);
    ui.label(RichText::new(crate::i18n::tr_args("updates-current", &[("version", env!("CARGO_PKG_VERSION").into())])).size(14.0).color(t.text_dim));
    ui.add_space(16.0);
    if let Some(release) = badge(app).cloned() {
        helder::card_frame(t).inner_margin(20).show(ui, |ui| {
            ui.set_width(ui.available_width().min(560.0));
            ui.horizontal(|ui| {
                helder::badge(ui, t, Icon::Download);
                ui.add_space(8.0);
                ui.label(
                    RichText::new(crate::i18n::tr_args("updates-available", &[("version", release.version.as_str().into())]))
                        .size(18.0)
                        .strong()
                        .color(t.text),
                );
            });
            if !release.notes.is_empty() {
                ui.add_space(12.0);
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| crate::release_notes::show(ui, t, &release.notes));
            }
            ui.add_space(12.0);
            // The window walks through the update, as when Merel offers it by itself.
            if helder::primary_button(ui, t, Icon::Download, &update()).clicked() {
                app.updates.offer = Some(release.clone());
            }
        });
        ui.add_space(16.0);
    }
    ui.horizontal(|ui| {
        let running = app.updates.running();
        let label = if running { tr("updates-checking-now") } else { tr("updates-check-now") };
        if ui.add_enabled(!running, egui::Button::new(RichText::new(label).size(14.0).color(t.text))).clicked() {
            app.check_for_updates(ui.ctx(), true);
        }
    });
    ui.add_space(12.0);
    let mut auto = app.view.update_check == Some(true);
    if helder::toggle(ui, t, &mut auto, &self::auto()).changed() {
        app.view.update_check = Some(auto);
        app.save_view(ui.ctx());
    }
    if app.updates.self_update {
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(tr("updates-page-installed")).size(12.0).color(t.text_dim)).wrap());
    }
}

/// A newer release this Merel can install itself must be installed before Merel can be used:
/// the update fills the main window, without "Later" or "Overslaan", so a fix for a
/// vulnerability reaches everyone. Closing Merel is still possible; the next start asks again.
pub fn required_screen(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, t.window);
    let Some(release) = app.updates.offer.clone() else { return };
    let width = 460.0_f32.min(ui.available_width() - 32.0);
    let mut close = false;
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(((ui.available_height() - 420.0) / 2.0).max(48.0));
            ui.label(RichText::new(window_label()).size(helder::TITLE_TEXT).strong().color(t.text));
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(width);
                if app.updates.job.is_some() {
                    close = progress(app, ui, t);
                } else {
                    offer(app, ui, t, &release);
                }
            });
        });
    });
    if app.updates.job.is_some() {
        end_progress(app, close);
    }
}

/// The offer: what is new, and the buttons. True when the person closed it.
fn offer(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens, release: &Release) -> bool {
    let required = app.updates.required;
    let here = install::Install::detect().filter(|_| install::SIGNING_KEY.is_some());
    let mut close = false;
    if here.is_some() || required {
        helder::step_dots(ui, t, 0, STEPS);
        ui.add_space(12.0);
    }
    helder::card_frame(t).inner_margin(20).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            helder::badge(ui, t, Icon::Download);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(crate::i18n::tr_args("updates-available", &[("version", release.version.as_str().into())]))
                        .size(18.0)
                        .strong()
                        .color(t.text),
                );
                ui.label(
                    RichText::new(crate::i18n::tr_args("updates-current", &[("version", env!("CARGO_PKG_VERSION").into())]))
                        .size(14.0)
                        .color(t.text_dim),
                );
            });
        });
        if required {
            ui.add_space(12.0);
            helder::paragraph(ui, t, &tr("updates-required-text"));
        }
        if !release.notes.is_empty() {
            ui.add_space(12.0);
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| crate::release_notes::show(ui, t, &release.notes));
        }
        ui.add_space(12.0);
        let how = if here.is_some() || required { tr("updates-how-update") } else { tr("updates-how-download") };
        ui.add(egui::Label::new(RichText::new(how).size(12.0).color(t.text_dim)).wrap());
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if !required {
            helder::quiet(ui, t, |ui| {
                if helder::text_button(ui, t, &skip()).on_hover_text(tr("updates-skip-hint")).clicked() {
                    app.view.skipped_version = Some(release.version.clone());
                    app.save_view(ui.ctx());
                    close = true;
                }
            });
        }
        helder::right(ui, |ui| {
            match (&here, install::SIGNING_KEY) {
                (Some(place), Some(key)) => {
                    if helder::primary_button(ui, t, Icon::Download, &update()).clicked() {
                        app.updates.job = Some(Job::start(ui.ctx(), place.clone(), &release.version, key));
                    }
                }
                // Only in tests: a required update is only ever found where Merel can install it.
                _ if required => {
                    helder::primary_button(ui, t, Icon::Download, &update());
                }
                _ => {
                    if helder::primary_button(ui, t, Icon::Download, &download()).clicked() {
                        open_page(app, release);
                        close = true;
                    }
                }
            }
            if !required {
                helder::secondary(ui, t, |ui| {
                    if ui.button(RichText::new(later()).size(14.0).color(t.text)).clicked() {
                        close = true;
                    }
                });
            }
        });
    });
    close
}

/// The steps the wizard shows as dots: offer, download, check, install.
const STEPS: usize = 4;

/// Ends Merel so the new version can start: closes the main window. The update window is a
/// window of its own, and a plain Close from inside it would shut only that one, leaving Merel
/// running and the new version waiting for it.
fn quit(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(quit_id(), true));
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
}

fn quit_id() -> egui::Id {
    egui::Id::new("merel-quitting")
}

/// Whether Merel itself asked its main window to close so that it ends (to start a new
/// version), rather than the person clicking the window's close button. On macOS the close
/// button only hides the window, and this close must not be held back like that.
pub fn quitting(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(quit_id()).unwrap_or(false))
}

/// The update under way: a progress bar while it downloads, then the check, the install and
/// the button that starts the new version; or what went wrong, with the release page. True
/// when the person closed it.
fn progress(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let Some(job) = &app.updates.job else { return false };
    let required = app.updates.required;
    let stage = job.stage();
    let version = job.version.clone();
    let release = Release { version: version.clone(), notes: String::new() };
    let (step, icon, title, text) = match &stage {
        Stage::Downloading { .. } => {
            (1, Icon::Download, crate::i18n::tr_args("updates-downloading", &[("version", version.as_str().into())]), String::new())
        }
        Stage::Checking => (2, Icon::ShieldCheck, tr("updates-checking"), tr("updates-checking-text")),
        Stage::Installing => {
            (3, Icon::RefreshCw, crate::i18n::tr_args("updates-installing", &[("version", version.as_str().into())]), tr("updates-installing-text"))
        }
        Stage::Ready(Restart::Installer(_)) => {
            (3, Icon::CircleCheck, crate::i18n::tr_args("updates-ready", &[("version", version.as_str().into())]), tr("updates-ready-text"))
        }
        Stage::Ready(Restart::Relaunch(_)) => {
            (3, Icon::CircleCheck, crate::i18n::tr_args("updates-installed", &[("version", version.as_str().into())]), tr("updates-installed-text"))
        }
        Stage::Failed(reason) => {
            (0, Icon::CircleAlert, tr("updates-failed"), crate::i18n::tr_args("updates-failed-text", &[("reason", reason.as_str().into())]))
        }
    };
    let mut close = false;
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
                Some(total) => crate::i18n::tr_args(
                    "updates-progress",
                    &[("done", format!("{:.0}", mb(done)).into()), ("total", format!("{:.0}", mb(total)).into())],
                ),
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
                    if ui.button(RichText::new(cancel()).size(14.0).color(t.text)).clicked() {
                        close = true;
                    }
                });
            }
            Stage::Checking | Stage::Installing => {}
            Stage::Ready(restart) => {
                if helder::primary_button(ui, t, Icon::RefreshCw, &restart_label()).clicked() {
                    let handover = install::hand_over(&app.engine);
                    match restart.start(handover.as_ref()) {
                        Ok(()) => quit(ui.ctx()),
                        Err(e) => {
                            log::warn!("could not start the new version: {e}");
                            if let Some(key) = &handover {
                                install::take_back(&app.engine, key);
                            }
                            app.notice = Some(Notice::error(tr("updates-restart-failed")));
                        }
                    }
                    close = true;
                }
                if !required {
                    helder::secondary(ui, t, |ui| {
                        if ui.button(RichText::new(later()).size(14.0).color(t.text)).clicked() {
                            close = true;
                        }
                    });
                }
            }
            Stage::Failed(_) => {
                if required {
                    if helder::primary_button(ui, t, Icon::RefreshCw, &retry()).clicked() {
                        close = true;
                    }
                } else if helder::primary_button(ui, t, Icon::Globe, &page_label()).clicked() {
                    open_page(app, &release);
                    close = true;
                }
                helder::secondary(ui, t, |ui| {
                    let (label, page) = if required { (page_label(), true) } else { (helder::close_label(), false) };
                    if ui.button(RichText::new(label).size(14.0).color(t.text)).clicked() {
                        if page {
                            open_page(app, &release);
                        } else {
                            close = true;
                        }
                    }
                });
            }
        });
    });
    close
}

/// Ends the update when the person closed it, unless it is installing: half an install must
/// not be walked away from. A required update goes back to its offer; any other closes.
fn end_progress(app: &mut MailApp, close: bool) {
    let Some(job) = &app.updates.job else { return };
    if !close || matches!(job.stage(), Stage::Checking | Stage::Installing) {
        return;
    }
    if let Some(job) = app.updates.job.take() {
        job.cancel();
    }
    if !app.updates.required {
        app.updates.offer = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_installed_merel_asks_again_after_an_hour_of_clock_time() {
        let ctx = egui::Context::default();
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.updates.self_update = true;
        app.updates.last = Some(SystemTime::now() - Duration::from_secs(50 * 60));
        app.tick_updates(&ctx);
        assert!(!app.updates.running(), "not yet within the hour");
        // Hours a laptop slept count as well: the clock on the wall decides.
        app.updates.last = Some(SystemTime::now() - INSTALLED_INTERVAL);
        app.tick_updates(&ctx);
        assert!(app.updates.running());
        // A clock set back does not stop the checks for good.
        app.updates.pending = None;
        app.updates.last = Some(SystemTime::now() + Duration::from_secs(3600));
        app.tick_updates(&ctx);
        assert!(app.updates.running());
    }

    /// Runs `inside` in a window of its own, as eframe does, and returns the commands meant for
    /// the main window.
    fn root_commands(inside: fn(&egui::Context)) -> Vec<egui::ViewportCommand> {
        use std::cell::RefCell;
        use std::rc::Rc;
        fn input(id: egui::ViewportId) -> egui::RawInput {
            egui::RawInput { viewport_id: id, viewports: std::iter::once((id, egui::ViewportInfo::default())).collect(), ..Default::default() }
        }
        let commands = Rc::new(RefCell::new(Vec::new()));
        let seen = commands.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            let mut out = ctx.run_ui(input(viewport.ids.this), |ui| (viewport.viewport_ui_cb)(ui));
            out.textures_delta.clear();
            seen.borrow_mut().extend(out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default());
        });
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut out = ctx.run_ui(input(egui::ViewportId::ROOT), |ui| {
            let builder = egui::ViewportBuilder::default().with_title("Bijwerken");
            ui.ctx().show_viewport_immediate(egui::ViewportId::from_hash_of("update"), builder, |ui, class| {
                assert!(class == egui::ViewportClass::Immediate, "a window of its own");
                inside(ui.ctx());
            });
        });
        out.textures_delta.clear();
        commands.borrow_mut().extend(out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default());
        commands.take()
    }

    #[test]
    fn starting_again_from_the_update_window_closes_merel_itself() {
        assert!(root_commands(quit).contains(&egui::ViewportCommand::Close));
        let ctx = egui::Context::default();
        assert!(!quitting(&ctx));
        quit(&ctx);
        assert!(quitting(&ctx), "the close button's hiding must let this close through");
        // What it did before: only the update window closed, and Merel kept running.
        assert!(!root_commands(|ctx| ctx.send_viewport_cmd(egui::ViewportCommand::Close)).contains(&egui::ViewportCommand::Close));
    }

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
