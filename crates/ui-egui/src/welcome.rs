//! The welcome at the first start: a short window in four steps (what Mailcraft is, an
//! account, updates, light or dark) that the person can leave at any step. It comes once per
//! computer: [`ViewPrefs::welcomed`](crate::view::ViewPrefs) is set when it closes, however it
//! closes. Every choice in it can be changed later in the same place as always (Bestand, Over
//! Mailcraft, Beeld).

use egui::RichText;

use crate::theme::Tokens;
use crate::view::ThemeChoice;
use crate::{MailApp, Tab};

/// The window's title and buttons, which tests and screen readers find them by.
pub const WINDOW: &str = "Welkom bij Mailcraft";
pub const NEXT: &str = "Volgende";
pub const BACK: &str = "Vorige";
pub const DONE: &str = "Aan de slag";
pub const SKIP: &str = "Overslaan";
pub const ADD_ACCOUNT: &str = "Account toevoegen";
pub const UPDATES_ON: &str = "Ja, elke dag kijken";
pub const UPDATES_OFF: &str = "Nee, ik kijk zelf via Help > Updates";

/// The steps, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Hello,
    Account,
    Updates,
    Look,
}

impl Step {
    const ALL: [Step; 4] = [Step::Hello, Step::Account, Step::Updates, Step::Look];

    fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Option<Step> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn back(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| Self::ALL[i])
    }
}

/// Whether the welcome opens at start: on a computer with settings, the first time.
pub fn due(app: &MailApp) -> bool {
    app.engine.config_path().is_some() && !app.view.welcomed
}

impl MailApp {
    /// Closes the welcome for good and saves the choices made in it.
    fn finish_welcome(&mut self, ctx: &egui::Context) {
        self.welcome = None;
        self.view.welcomed = true;
        if self.view.update_check.is_none() {
            self.view.update_check = Some(false);
        }
        self.save_view(ctx);
    }
}

/// The welcome window, while it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(step) = app.welcome else { return };
    let mut go: Option<Option<Step>> = None;
    let mut finish = false;
    egui::Window::new(WINDOW)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(RichText::new(format!("Stap {} van {}", step.index() + 1, Step::ALL.len())).size(12.0).color(t.text_dim));
            ui.add_space(4.0);
            match step {
                Step::Hello => {
                    ui.label(RichText::new("Mail, agenda, contacten en taken op één plek").size(18.0).strong().color(t.text));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Mailcraft werkt zoals Outlook en draait op je eigen computer. Je mail gaat rechtstreeks tussen deze computer en je mailserver; er zit geen dienst van ons tussen.").color(t.text));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Afbeeldingen van internet laden pas als je erom vraagt, wachtwoorden staan in de sleutelhanger van het systeem en de offline kopie van je mail is versleuteld.").color(t.text));
                }
                Step::Account => {
                    ui.label(RichText::new("Je mailaccount").size(18.0).strong().color(t.text));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Voeg je account toe met IMAP en SMTP, Gmail, Microsoft 365 of Exchange. Dat kan ook later, onder Bestand.").color(t.text));
                    ui.add_space(8.0);
                    if ui.button(ADD_ACCOUNT).clicked() {
                        app.tab = Tab::File;
                        finish = true;
                    }
                }
                Step::Updates => {
                    ui.label(RichText::new("Nieuwe versies").size(18.0).strong().color(t.text));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Mailcraft kan eens per dag bij GitHub nagaan of er een nieuwe versie is en het je dan laten weten. Er gaat niets mee behalve die vraag; GitHub ziet wel het adres van je internetverbinding.").color(t.text));
                    ui.add_space(8.0);
                    let mut on = app.view.update_check == Some(true);
                    let before = on;
                    ui.radio_value(&mut on, true, UPDATES_ON);
                    ui.radio_value(&mut on, false, UPDATES_OFF);
                    if on != before || app.view.update_check.is_none() {
                        app.view.update_check = Some(on);
                    }
                }
                Step::Look => {
                    ui.label(RichText::new("Licht of donker").size(18.0).strong().color(t.text));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Dit kun je later wijzigen op het tabblad Beeld.").color(t.text));
                    ui.add_space(8.0);
                    for choice in ThemeChoice::ALL {
                        if ui.radio(app.view.theme == choice, choice.dutch_name()).on_hover_text(choice.hint()).clicked() {
                            app.view.theme = choice;
                            ctx.set_theme(choice.preference());
                        }
                    }
                }
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if let Some(back) = step.back()
                    && ui.button(BACK).clicked()
                {
                    go = Some(Some(back));
                }
                match step.next() {
                    Some(next) => {
                        if ui.button(NEXT).clicked() {
                            go = Some(Some(next));
                        }
                        if ui.button(SKIP).on_hover_text("De rest later instellen").clicked() {
                            finish = true;
                        }
                    }
                    None => finish |= ui.button(DONE).clicked(),
                }
            });
        });
    if finish {
        app.finish_welcome(ctx);
    } else if let Some(Some(next)) = go {
        app.welcome = Some(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_run_in_order() {
        assert_eq!(Step::Hello.back(), None);
        assert_eq!(Step::Hello.next(), Some(Step::Account));
        assert_eq!(Step::Look.next(), None);
        assert_eq!(Step::Look.back(), Some(Step::Updates));
    }
}
