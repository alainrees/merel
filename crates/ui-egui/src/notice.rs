//! What the app says after an action, as the new Outlook does: a message bar at the top of
//! the window, just under the commands the person used, with an icon and a colour for its kind
//! (done, for your information, take care, went wrong). Good news and information go away on
//! their own after a few seconds (not while the pointer rests on them); a warning stays longer
//! and an error stays until it is closed. A saved file names only itself, never the full path,
//! and offers to show it in Finder (Explorer, the file manager).

use std::ops::Deref;
use std::path::{Path, PathBuf};

use egui::{Align2, Color32, Margin, RichText, Stroke, vec2};

use crate::MailApp;
use crate::helder::{self, Icon};
use crate::theme::Tokens;

/// The kind of a notice, which sets its icon and colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// The action worked: a file was saved, a message sent.
    Success,
    /// Something the person may like to know, that needs nothing from them.
    Info,
    /// The action did not happen as asked, for a reason the person can mend.
    Warning,
    /// The action failed.
    Error,
}

impl Tone {
    pub fn icon(self) -> Icon {
        match self {
            Tone::Success => Icon::CircleCheck,
            Tone::Info => Icon::Info,
            Tone::Warning => Icon::TriangleAlert,
            Tone::Error => Icon::CircleAlert,
        }
    }

    /// The colour of the icon and the bar at the notice's edge.
    pub fn color(self, t: &Tokens) -> Color32 {
        match self {
            Tone::Success => t.success,
            Tone::Info => t.accent,
            Tone::Warning => t.warning,
            Tone::Error => t.flag,
        }
    }

    /// What a screen reader says before the text, as the icon's colour does not reach it.
    pub fn spoken(self) -> &'static str {
        match self {
            Tone::Success => "Gelukt",
            Tone::Info => "Informatie",
            Tone::Warning => "Let op",
            Tone::Error => "Fout",
        }
    }

    /// How many seconds the notice stays; `None` until it is closed.
    pub fn lasts(self) -> Option<f64> {
        match self {
            Tone::Success | Tone::Info => Some(6.0),
            Tone::Warning => Some(12.0),
            Tone::Error => None,
        }
    }
}

/// One notice: its kind, its text and, for a saved file, where the file went.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub tone: Tone,
    pub text: String,
    /// A file or folder to show in the system's file manager, from the notice's button.
    pub reveal: Option<PathBuf>,
    /// When the notice was first drawn, for the countdown; reset while the pointer is on it.
    shown: Option<f64>,
}

impl Notice {
    pub fn new(tone: Tone, text: impl Into<String>) -> Self {
        Notice { tone, text: text.into(), reveal: None, shown: None }
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self::new(Tone::Success, text)
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self::new(Tone::Info, text)
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(Tone::Warning, text)
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(Tone::Error, text)
    }

    /// The notice with a button that shows `path` in the file manager.
    pub fn revealing(mut self, path: impl Into<PathBuf>) -> Self {
        self.reveal = Some(path.into());
        self
    }
}

/// A notice reads as its text, so `notice.as_deref()` gives the words.
impl Deref for Notice {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

/// What a saved file is called in a notice: its name alone, not the folders above it.
pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The folder a file went into, by its own name ("Downloads").
pub fn folder_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string())
}

/// The button that shows a saved file, named after the system's file manager.
pub const REVEAL: &str = if cfg!(target_os = "macos") {
    "Toon in Finder"
} else if cfg!(windows) {
    "Toon in Verkenner"
} else {
    "Toon in map"
};

/// The close button's accessible name.
pub const CLOSE: &str = "Melding sluiten";

/// How far below the commands the notice floats, and how wide it may grow.
const GAP: f32 = 8.0;
const MAX_WIDTH: f32 = 560.0;

/// Draws the notice, if there is one, centred at `top`: the edge of the content under the
/// ribbon. Counts down a notice that goes by itself.
pub fn toast(app: &mut MailApp, ctx: &egui::Context, t: &Tokens, top: f32) {
    let Some(notice) = app.notice.as_mut() else { return };
    let now = ctx.input(|i| i.time);
    let shown = *notice.shown.get_or_insert(now);
    if let Some(life) = notice.tone.lasts() {
        let left = life - (now - shown);
        if left <= 0.0 {
            app.notice = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    let tone = notice.tone;
    let color = tone.color(t);
    let text = notice.text.clone();
    let reveal = notice.reveal.clone();
    let mut close = false;
    let mut show_file = false;
    let area = egui::Area::new(egui::Id::new("notice"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, top + GAP))
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_max_width(MAX_WIDTH);
            egui::Frame::new()
                .fill(t.card)
                // The kind's colour around the edge, besides its icon.
                .stroke(Stroke::new(1.0, color))
                .corner_radius(helder::CARD_RADIUS)
                .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if t.dark { 90 } else { 28 }) })
                .inner_margin(Margin { left: 14, right: 6, top: 6, bottom: 6 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(helder::BUTTON_HEIGHT);
                        ui.add(tone.icon().image(helder::ICON, color));
                        let label = ui.add(egui::Label::new(RichText::new(&text).size(14.0).color(t.text)).wrap());
                        // Read out as it appears, an error at once, the rest when the reader is free.
                        let live = if tone == Tone::Error { egui::accesskit::Live::Assertive } else { egui::accesskit::Live::Polite };
                        ui.ctx().accesskit_node_builder(label.id, |node| {
                            node.set_live(live);
                            node.set_label(format!("{}: {text}", tone.spoken()));
                        });
                        if reveal.is_some() && helder::quiet(ui, t, |ui| helder::tool_button(ui, t, Icon::FolderOpen, REVEAL)).clicked() {
                            show_file = true;
                        }
                        if helder::quiet(ui, t, |ui| helder::icon_button(ui, t, Icon::X, CLOSE)).clicked() {
                            close = true;
                        }
                    });
                });
        });
    // The countdown waits while the pointer rests on the notice, so it can be read in peace.
    if area.response.contains_pointer()
        && let Some(n) = app.notice.as_mut()
    {
        n.shown = Some(now);
    }
    if show_file && let Some(path) = reveal {
        if let Err(e) = show_in_file_manager(&path) {
            log::warn!("could not show {}: {e}", path.display());
            app.notice = Some(Notice::error(format!("{REVEAL} is niet gelukt.")));
        }
        return;
    }
    if close {
        app.notice = None;
    }
}

/// The notice inside a page, for a message that belongs with what is on the page (the account
/// list): the same icon, colour and text, without the countdown.
pub fn inline(ui: &mut egui::Ui, t: &Tokens, notice: &Notice) {
    let color = notice.tone.color(t);
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, color))
        .corner_radius(helder::CONTROL_RADIUS)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(notice.tone.icon().image(helder::ICON, color));
                let label = ui.add(egui::Label::new(RichText::new(&notice.text).size(14.0).color(t.text)).wrap());
                ui.ctx().accesskit_node_builder(label.id, |node| node.set_label(format!("{}: {}", notice.tone.spoken(), notice.text)));
            });
        });
}

/// Shows `path` in the system's file manager: a file selected in its folder, a folder opened.
fn show_in_file_manager(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        return open::that_detached(path);
    }
    #[cfg(target_os = "macos")]
    {
        let mut child = std::process::Command::new("/usr/bin/open").arg("-R").arg(path).spawn()?;
        std::thread::spawn(move || child.wait());
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer reads its own quoting; a Windows file name holds no quote of its own.
        let mut child = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn()?;
        std::thread::spawn(move || child.wait());
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        // File managers on Linux differ in how they select a file; the folder opens.
        match path.parent() {
            Some(dir) => open::that_detached(dir),
            None => open::that_detached(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_errors_wait_to_be_closed() {
        assert!(Tone::Success.lasts().is_some());
        assert!(Tone::Info.lasts().is_some());
        assert!(Tone::Warning.lasts() > Tone::Info.lasts());
        assert_eq!(Tone::Error.lasts(), None);
    }

    #[test]
    fn every_kind_has_its_own_icon() {
        let icons = [Tone::Success, Tone::Info, Tone::Warning, Tone::Error].map(Tone::icon);
        for (i, a) in icons.iter().enumerate() {
            assert!(icons[i + 1..].iter().all(|b| b != a), "{a:?} twice");
        }
    }

    #[test]
    fn a_saved_file_is_named_without_its_folders() {
        let path = Path::new("/Users/someone/Downloads/Ravi Kumar (2).vcf");
        assert_eq!(file_name(path), "Ravi Kumar (2).vcf");
        assert_eq!(folder_name(path.parent().unwrap()), "Downloads");
    }
}
