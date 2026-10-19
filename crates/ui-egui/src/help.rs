//! The "Help" tab: the keyboard shortcuts in one list, the folder with this computer's settings,
//! the project page, and "Over Mailcraft" with the version and what the app stands for.

use egui::{RichText, vec2};

use crate::{MailApp, Tokens, key_hint, ribbon_action_hint};

/// The project page, for the "Op GitHub" button.
pub const REPOSITORY: &str = "https://github.com/alainrees/mailcraft";

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
        (key_hint("F"), "Doorsturen"),
        (key_hint("Shift+V"), "Verplaatsen naar een map"),
        (key_hint("P"), "Afdrukken via de browser"),
        (key_hint("Q"), "Markeren als gelezen"),
        (key_hint("U"), "Markeren als ongelezen"),
        (key_hint("E"), "Naar het zoekveld (ook in agenda, contacten en taken)"),
        (key_hint("Shift+1") + " … 9", "Snelle stap 1 tot 9 van het account"),
        ("Esc".to_string(), "Zoekopdracht wissen (ook in agenda, contacten en taken)"),
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
        app.notice = Some(format!("Openen is niet gelukt: {target}"));
    }
}

/// The "Help" tab.
pub fn ribbon(app: &mut MailApp, ui: &mut egui::Ui, t: &Tokens) {
    if ribbon_action_hint(ui, t, "⌨", "Sneltoetsen", true, Some("Alle toetsen van de mailmodule op een rij")) {
        app.help = Some(HelpWindow::Shortcuts);
    }
    ui.separator();
    let folder = settings_folder(app);
    if ribbon_action_hint(ui, t, "📂", "Instellingenmap", folder.is_some(), Some("De map met accounts, agenda, regels en cache openen"))
        && let Some(f) = folder
    {
        open_outside(app, &f.to_string_lossy());
    }
    if ribbon_action_hint(ui, t, "🌐", "Op GitHub", true, Some("De projectpagina met de handleiding openen in de browser")) {
        open_outside(app, REPOSITORY);
    }
    ui.separator();
    if ribbon_action_hint(ui, t, "ℹ", "Over Mailcraft", true, Some("Versie en uitgangspunten")) {
        app.help = Some(HelpWindow::About);
    }
}

/// The open help window, if any.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(which) = app.help else { return };
    let mut close = false;
    match which {
        HelpWindow::Shortcuts => {
            egui::Window::new("Sneltoetsen").collapsible(false).resizable(false).default_width(380.0).show(ctx, |ui| {
                ui.label(RichText::new("In de mailmodule, zolang je niet in een tekstveld typt.").size(12.0).color(t.text_dim));
                ui.add_space(6.0);
                egui::Grid::new("shortcuts").num_columns(2).spacing(vec2(18.0, 6.0)).show(ui, |ui| {
                    for (keys, what) in shortcut_list() {
                        ui.label(RichText::new(keys).strong().color(t.text));
                        ui.label(RichText::new(what).color(t.text));
                        ui.end_row();
                    }
                });
                ui.add_space(8.0);
                close = ui.button("Sluiten").clicked();
            });
        }
        HelpWindow::About => {
            egui::Window::new("Over Mailcraft").collapsible(false).resizable(false).default_width(400.0).show(ctx, |ui| {
                ui.label(RichText::new("Mailcraft").size(22.0).strong().color(t.text));
                ui.label(RichText::new(format!("Versie {}", env!("CARGO_PKG_VERSION"))).color(t.text_dim));
                ui.add_space(8.0);
                ui.label(RichText::new("Mail, agenda, contacten en taken voor de desktop, met de werkwijze van Outlook en de stijl van de ArtCraft-familie. Geschreven in Rust met egui; één programma voor macOS, Windows en Linux.").color(t.text));
                ui.add_space(8.0);
                ui.label(RichText::new("Veiligheid en privacy voorop: mail kan geen scripts uitvoeren, afbeeldingen van internet worden pas geladen als je erom vraagt, verbindingen lopen altijd over TLS, wachtwoorden staan in de sleutelhanger van het systeem en de offline cache is versleuteld.").color(t.text));
                if let Some(f) = settings_folder(app) {
                    ui.add_space(8.0);
                    ui.label(RichText::new(format!("Instellingen en cache: {}", f.display())).size(12.0).color(t.text_dim));
                }
                ui.add_space(8.0);
                close = ui.button("Sluiten").clicked();
            });
        }
    }
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
        assert_eq!(list.len(), 22);
        assert!(list.iter().any(|(k, _)| k.ends_with("+Shift+V")));
        assert!(list.iter().all(|(k, what)| !k.is_empty() && !what.is_empty()));
    }
}
