//! The "Help" tab: the keyboard shortcuts in one list, the folder with this computer's settings,
//! the project page, and "Over Merel" with the version and what the app stands for.

use crate::i18n::tr;
use egui::{RichText, vec2};

use crate::helder::{self, Icon};
use crate::{MailApp, Tokens, key_hint, ribbon_action_hint};

/// The button in "Over Merel" that shows the folder with this computer's settings.
pub fn open_settings() -> String {
    tr("help-open-settings")
}

/// The project page, for the "Op GitHub" button.
pub const REPOSITORY: &str = "https://github.com/alainrees/merel";

/// The version of this build, as "Over Merel" and the Help tab show it: the release number
/// plus the commit and its date, so two builds of the same release can be told apart.
pub fn version_line() -> String {
    version_text(env!("CARGO_PKG_VERSION"), env!("MEREL_COMMIT"), env!("MEREL_COMMIT_DATE"))
}

fn version_text(version: &str, commit: &str, date: &str) -> String {
    match (commit.is_empty(), date.is_empty()) {
        (true, _) => crate::i18n::tr_args("version-plain", &[("version", version.into())]),
        (false, true) => crate::i18n::tr_args("version-build", &[("version", version.into()), ("commit", commit.into())]),
        (false, false) => {
            crate::i18n::tr_args("version-build-date", &[("version", version.into()), ("commit", commit.into()), ("date", date.into())])
        }
    }
}

/// Which help window is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpWindow {
    Shortcuts,
    About,
}

/// The shortcuts of the mail module, as the "Sneltoetsen" window lists them: the keys (with
/// Cmd or Ctrl for this computer) and what they do.
pub fn shortcut_list() -> Vec<(String, String)> {
    vec![
        ("↑ / ↓".to_string(), tr("shortcut-1")),
        (tr("key-enter-double-click"), tr("shortcut-2")),
        (format!("{} / Shift+{}", key_hint(&tr("key-click")), tr("key-click")), tr("shortcut-3")),
        (key_hint("A"), tr("shortcut-4")),
        ("Delete".to_string(), tr("shortcut-5")),
        ("Ctrl+Delete".to_string(), tr("shortcut-6")),
        (if cfg!(target_os = "macos") { "Ctrl+E" } else { "Backspace" }.to_string(), tr("shortcut-7")),
        (key_hint("N"), tr("shortcut-8")),
        (key_hint("Shift+M"), tr("shortcut-9")),
        (key_hint("Shift+A"), tr("shortcut-10")),
        (key_hint("Shift+C"), tr("shortcut-11")),
        (key_hint("Shift+K"), tr("shortcut-12")),
        (key_hint("R"), tr("shortcut-13")),
        (key_hint("Shift+R"), tr("shortcut-14")),
        (key_hint("Alt+R"), tr("shortcut-15")),
        (key_hint("F"), tr("shortcut-16")),
        (key_hint("Alt+F"), tr("shortcut-17")),
        (key_hint("Shift+V"), tr("shortcut-18")),
        (key_hint("P"), tr("shortcut-19")),
        (key_hint("S"), tr("shortcut-20")),
        (key_hint("G"), tr("shortcut-21")),
        (key_hint("Q"), tr("shortcut-22")),
        (key_hint("U"), tr("shortcut-23")),
        (key_hint("E"), tr("shortcut-24")),
        (tr("search-keywords"), tr("shortcut-25")),
        (key_hint("Shift+1") + " … 9", tr("shortcut-26")),
        ("Esc".to_string(), tr("shortcut-27")),
        ("F5".to_string(), tr("shortcut-28")),
    ]
}

/// The folder that holds the accounts file and the other local files, if the app has one.
fn settings_folder(app: &MailApp) -> Option<std::path::PathBuf> {
    app.engine.config_path().and_then(|p| p.parent()).map(std::path::Path::to_path_buf)
}

/// Hands a path or address to the system to open; a failure becomes a notice.
fn open_outside(app: &mut MailApp, target: &str) {
    if let Err(e) = open::that_detached(target) {
        log::warn!("could not open {target}: {e}");
        app.notice = Some(crate::Notice::error(crate::i18n::tr_args("updates-open-failed", &[("page", target.into())])));
    }
}

/// The "Help" tab.
pub fn ribbon(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    if ribbon_action_hint(ui, t, "⌨", &tr("help-shortcuts"), true, Some(&tr("help-shortcuts-hint"))) {
        app.help = Some(HelpWindow::Shortcuts);
    }
    crate::ribbon_separator(ui);
    let folder = settings_folder(app);
    if ribbon_action_hint(ui, t, "📂", &tr("help-settings-folder"), folder.is_some(), Some(&tr("help-settings-folder-hint")))
        && let Some(f) = folder
    {
        open_outside(app, &f.to_string_lossy());
    }
    if ribbon_action_hint(ui, t, "🌐", &tr("help-github"), true, Some(&tr("help-github-hint"))) {
        open_outside(app, REPOSITORY);
    }
    crate::ribbon_separator(ui);
    if ribbon_action_hint(ui, t, "ℹ", &tr("help-about"), true, Some(&tr("help-about-hint"))) {
        app.help = Some(HelpWindow::About);
    }
    let checking = app.updates.running();
    if ribbon_action_hint(ui, t, "⟳", &tr("help-updates"), !checking, Some(&tr("help-updates-hint"))) {
        app.check_for_updates(ui.ctx(), true);
    }
    crate::ribbon_separator(ui);
    ui.label(RichText::new(version_line()).size(12.0).color(t.text_dim));
}

/// The open help window, if any.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(which) = app.help else { return };
    let close = match which {
        HelpWindow::Shortcuts => helder::dialog(ctx, t, &tr("help-shortcuts"), 560.0, |ui| {
            ui.label(RichText::new(tr("help-shortcuts-where")).size(12.0).color(t.text_dim));
            ui.add_space(6.0);
            helder::card_frame(t).inner_margin(16).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                    egui::Grid::new("shortcuts").num_columns(2).spacing(vec2(18.0, 8.0)).show(ui, |ui| {
                        for (keys, what) in shortcut_list() {
                            ui.label(RichText::new(keys).size(14.0).strong().color(t.text));
                            ui.add(egui::Label::new(RichText::new(what).size(14.0).color(t.text)).wrap());
                            ui.end_row();
                        }
                    });
                });
            });
        }),
        HelpWindow::About => helder::dialog(ctx, t, &tr("help-about"), 460.0, |ui| {
            helder::card_frame(t).inner_margin(20).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    helder::app_badge(ui);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Merel").size(18.0).strong().color(t.text));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(version_line()).size(14.0).color(t.text_dim));
                            helder::quiet(ui, t, |ui| {
                                if ui
                                    .add(egui::Button::new(RichText::new(tr("help-copy")).size(12.0).color(t.accent_text)).frame_when_inactive(false))
                                    .on_hover_text(tr("help-copy-hint"))
                                    .clicked()
                                {
                                    ui.ctx().copy_text(format!("Merel {}", version_line()));
                                }
                            });
                        });
                    });
                });
                ui.add_space(12.0);
                helder::paragraph(ui, t, &tr("help-about-1"));
                ui.add_space(8.0);
                helder::paragraph(ui, t, &tr("help-about-2"));
                ui.add_space(8.0);
                let mut auto = app.view.update_check == Some(true);
                if helder::toggle(ui, t, &mut auto, &crate::updates::auto()).on_hover_text(tr("help-auto-hint")).changed() {
                    app.view.update_check = Some(auto);
                    app.save_view(ctx);
                }
                if let Some(f) = settings_folder(app) {
                    helder::quiet(ui, t, |ui| {
                        if helder::tool_button(ui, t, Icon::FolderOpen, &open_settings()).on_hover_text(tr("help-settings-hint")).clicked() {
                            open_outside(app, &f.to_string_lossy());
                        }
                    });
                }
            });
        }),
    };
    if close {
        app.help = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_list_names_every_key_the_mail_module_takes() {
        let list = shortcut_list();
        assert_eq!(list.len(), 28);
        assert!(list.iter().any(|(k, _)| k.ends_with("+Shift+V")));
        assert!(list.iter().all(|(k, what)| !k.is_empty() && !what.is_empty()));
    }

    #[test]
    fn the_version_names_the_build_when_it_is_known() {
        assert_eq!(version_text("0.1.0", "", ""), "Versie 0.1.0");
        assert_eq!(version_text("0.1.0", "abc1234", ""), "Versie 0.1.0 (build abc1234)");
        assert_eq!(version_text("0.1.0", "abc1234", "2026-10-20"), "Versie 0.1.0 (build abc1234 van 2026-10-20)");
        assert!(version_line().starts_with(&format!("Versie {}", env!("CARGO_PKG_VERSION"))));
    }
}
