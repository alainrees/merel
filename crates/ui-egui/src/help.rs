//! The "Help" tab: the keyboard shortcuts in one list, the folder with this computer's settings,
//! the project page, and "Over Merel" with the version and what the app stands for.

use egui::{RichText, vec2};

use crate::helder::{self, Icon};
use crate::{MailApp, Tokens, key_hint, ribbon_action_hint};

/// The button in "Over Merel" that shows the folder with this computer's settings.
pub const OPEN_SETTINGS: &str = "Instellingenmap openen";

/// The project page, for the "Op GitHub" button.
pub const REPOSITORY: &str = "https://github.com/alainrees/merel";

/// The version of this build, as "Over Merel" and the Help tab show it: the release number
/// plus the commit and its date, so two builds of the same release can be told apart.
pub fn version_line() -> String {
    version_text(env!("CARGO_PKG_VERSION"), env!("MEREL_COMMIT"), env!("MEREL_COMMIT_DATE"))
}

fn version_text(version: &str, commit: &str, date: &str) -> String {
    match (commit.is_empty(), date.is_empty()) {
        (true, _) => format!("Versie {version}"),
        (false, true) => format!("Versie {version} (build {commit})"),
        (false, false) => format!("Versie {version} (build {commit} van {date})"),
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
pub fn shortcut_list() -> Vec<(String, &'static str)> {
    vec![
        ("↑ / ↓".to_string(), "Vorig of volgend bericht in de lijst"),
        ("Enter / dubbelklik".to_string(), "Geselecteerd bericht in een eigen venster openen"),
        (format!("{} / Shift+klik", key_hint("klik")), "Meer berichten selecteren, los of als reeks; de acties gelden dan voor alle"),
        (key_hint("A"), "Alle berichten in de lijst selecteren"),
        ("Delete".to_string(), "Geselecteerd bericht verwijderen"),
        ("Ctrl+Delete".to_string(), "Gesprek negeren: dit en volgende berichten erin naar Verwijderde items"),
        (if cfg!(target_os = "macos") { "Ctrl+E" } else { "Backspace" }.to_string(), "Geselecteerd bericht archiveren"),
        (key_hint("N"), "Nieuwe e-mail"),
        (key_hint("Shift+M"), "Nieuwe e-mail, vanuit elke module"),
        (key_hint("Shift+A"), "Nieuwe afspraak, vanuit elke module"),
        (key_hint("Shift+C"), "Nieuw contact, vanuit elke module"),
        (key_hint("Shift+K"), "Nieuwe taak, vanuit elke module"),
        (key_hint("R"), "Beantwoorden"),
        (key_hint("Shift+R"), "Allen beantwoorden"),
        (key_hint("Alt+R"), "Beantwoorden met vergaderverzoek"),
        (key_hint("F"), "Doorsturen"),
        (key_hint("Alt+F"), "Doorsturen als bijlage (het bericht gaat mee als .eml-bestand)"),
        (key_hint("Shift+V"), "Verplaatsen naar een map"),
        (key_hint("P"), "Afdrukken via de browser (in de agenda: de open dag, week of maand)"),
        (key_hint("S"), "Opslaan als .eml-bestand in de map Downloads"),
        (key_hint("G"), "Ga naar datum (in de agenda)"),
        (key_hint("Q"), "Markeren als gelezen"),
        (key_hint("U"), "Markeren als ongelezen"),
        (key_hint("E"), "Naar het zoekveld (ook in agenda, contacten en taken)"),
        ("van: aan: onderwerp:".to_string(), "Zoekwoorden in het zoekveld, zoals in Outlook; ook heeft:bijlage, is:ongelezen en is:gemarkeerd"),
        (key_hint("Shift+1") + " … 9", "Snelle stap 1 tot 9 van het account"),
        ("Esc".to_string(), "Berichtvenster sluiten, anders zoekopdracht wissen (ook in agenda, contacten en taken)"),
        ("F5".to_string(), "Map vernieuwen"),
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
        app.notice = Some(crate::Notice::error(format!("Openen is niet gelukt: {target}")));
    }
}

/// The "Help" tab.
pub fn ribbon(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    if ribbon_action_hint(ui, t, "⌨", "Sneltoetsen", true, Some("Alle toetsen van de mailmodule op een rij")) {
        app.help = Some(HelpWindow::Shortcuts);
    }
    crate::ribbon_separator(ui);
    let folder = settings_folder(app);
    if ribbon_action_hint(ui, t, "📂", "Instellingenmap", folder.is_some(), Some("De map met accounts, agenda, regels en cache openen"))
        && let Some(f) = folder
    {
        open_outside(app, &f.to_string_lossy());
    }
    if ribbon_action_hint(ui, t, "🌐", "Op GitHub", true, Some("De projectpagina met de handleiding openen in de browser")) {
        open_outside(app, REPOSITORY);
    }
    crate::ribbon_separator(ui);
    if ribbon_action_hint(ui, t, "ℹ", "Over Merel", true, Some("Versie en uitgangspunten")) {
        app.help = Some(HelpWindow::About);
    }
    let checking = app.updates.running();
    if ribbon_action_hint(ui, t, "⟳", crate::updates::CHECK, !checking, Some("Bij GitHub nagaan of er een nieuwere versie van Merel is")) {
        app.check_for_updates(ui.ctx(), true);
    }
    crate::ribbon_separator(ui);
    ui.label(RichText::new(version_line()).size(12.0).color(t.text_dim));
}

/// The open help window, if any.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(which) = app.help else { return };
    let close = match which {
        HelpWindow::Shortcuts => helder::dialog(ctx, t, "Sneltoetsen", 560.0, |ui| {
            ui.label(RichText::new("In de mailmodule, zolang je niet in een tekstveld typt.").size(12.0).color(t.text_dim));
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
        HelpWindow::About => helder::dialog(ctx, t, "Over Merel", 460.0, |ui| {
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
                                        .add(egui::Button::new(RichText::new("Kopiëren").size(12.0).color(t.accent_text)).frame_when_inactive(false))
                                        .on_hover_text("De versie naar het klembord, handig bij een foutmelding")
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(format!("Merel {}", version_line().trim_start_matches("Versie ")));
                                    }
                                });
                            });
                        });
                    });
                    ui.add_space(12.0);
                    helder::paragraph(ui, t, "Mail, agenda, contacten en taken voor de desktop, met de werkwijze van Outlook. Geschreven in Rust met egui; één programma voor macOS, Windows en Linux.");
                    ui.add_space(8.0);
                    helder::paragraph(ui, t, "Veiligheid en privacy voorop: mail kan geen scripts uitvoeren, afbeeldingen van internet worden pas geladen als je erom vraagt, verbindingen lopen altijd over TLS, wachtwoorden staan in de sleutelhanger van het systeem en de offline cache is versleuteld.");
                    ui.add_space(8.0);
                    let mut auto = app.view.update_check == Some(true);
                    if helder::toggle(ui, t, &mut auto, crate::updates::AUTO)
                        .on_hover_text("Merel vraagt dan eens per dag bij GitHub naar de nieuwste versie; er gaat niets anders mee dan die vraag")
                        .changed()
                    {
                        app.view.update_check = Some(auto);
                        app.save_view(ctx);
                    }
                    if let Some(f) = settings_folder(app) {
                        helder::quiet(ui, t, |ui| {
                            if helder::tool_button(ui, t, Icon::FolderOpen, OPEN_SETTINGS)
                                .on_hover_text("De map met je accounts, agenda, regels en de versleutelde cache")
                                .clicked()
                            {
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
