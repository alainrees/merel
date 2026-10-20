//! The welcome at the first start: a short window in four steps (what Mailcraft is, an
//! account, updates, light or dark) that the person can leave at any step. It comes once per
//! computer: [`ViewPrefs::welcomed`](crate::view::ViewPrefs) is set when it closes, however it
//! closes. Every choice in it can be changed later in the same place as always (Bestand, Over
//! Mailcraft, Beeld).

use egui::RichText;

use crate::helder::{self, Icon};
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
pub const UPDATES_ON: &str = "Elke dag kijken of er een nieuwe versie is";

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

impl Step {
    /// The icon at the head of the step's card.
    fn icon(self) -> Icon {
        match self {
            Step::Hello => Icon::Sparkles,
            Step::Account => Icon::Mail,
            Step::Updates => Icon::RefreshCw,
            Step::Look => Icon::Sun,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Step::Hello => "Mail, agenda, contacten en taken op één plek",
            Step::Account => "Je mailaccount",
            Step::Updates => "Nieuwe versies",
            Step::Look => "Licht of donker",
        }
    }
}

/// The welcome window, while it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(step) = app.welcome else { return };
    let mut go: Option<Step> = None;
    let mut finish = false;
    let closed = helder::dialog(ctx, t, WINDOW, 480.0, |ui| {
        helder::card_frame(t).inner_margin(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                helder::badge(ui, t, step.icon());
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    helder::step_dots(ui, t, step.index(), Step::ALL.len());
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(step.title()).size(18.0).strong().color(t.text)).wrap());
                });
            });
            ui.add_space(12.0);
            match step {
                Step::Hello => {
                    helder::paragraph(ui, t, "Mailcraft werkt zoals Outlook en draait op je eigen computer. Je mail gaat rechtstreeks tussen deze computer en je mailserver; er zit geen dienst van ons tussen.");
                    ui.add_space(6.0);
                    helder::paragraph(ui, t, "Afbeeldingen van internet laden pas als je erom vraagt, wachtwoorden staan in de sleutelhanger van het systeem en de offline kopie van je mail is versleuteld.");
                }
                Step::Account => {
                    helder::paragraph(ui, t, "Voeg je account toe met IMAP en SMTP, Gmail, Microsoft 365 of Exchange. Dat kan ook later, onder Bestand.");
                    ui.add_space(10.0);
                    helder::secondary(ui, t, |ui| {
                        if ui.add(egui::Button::image_and_text(Icon::UserPlus.image(helder::ICON, t.text), RichText::new(ADD_ACCOUNT).size(14.0).color(t.text))).clicked() {
                            app.tab = Tab::File;
                            finish = true;
                        }
                    });
                }
                Step::Updates => {
                    helder::paragraph(ui, t, "Mailcraft kan eens per dag bij GitHub nagaan of er een nieuwe versie is en het je dan laten weten. Er gaat niets mee behalve die vraag; GitHub ziet wel het adres van je internetverbinding.");
                    ui.add_space(10.0);
                    let mut on = app.view.update_check == Some(true);
                    helder::toggle(ui, t, &mut on, UPDATES_ON);
                    app.view.update_check = Some(on);
                    ui.add(egui::Label::new(RichText::new("Zelf kijken kan altijd via Help > Updates.").size(12.0).color(t.text_dim)).wrap());
                }
                Step::Look => {
                    helder::paragraph(ui, t, "Dit kun je later wijzigen op het tabblad Beeld.");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        helder::quiet(ui, t, |ui| {
                            for (choice, icon) in [(ThemeChoice::System, Icon::Monitor), (ThemeChoice::Light, Icon::Sun), (ThemeChoice::Dark, Icon::Moon)] {
                                if helder::toggle_button(ui, t, icon, choice.dutch_name(), app.view.theme == choice).on_hover_text(choice.hint()).clicked() {
                                    app.view.theme = choice;
                                    ctx.set_theme(choice.preference());
                                }
                            }
                        });
                    });
                }
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if step.next().is_some() {
                helder::quiet(ui, t, |ui| {
                    if helder::text_button(ui, t, SKIP).on_hover_text("De rest later instellen").clicked() {
                        finish = true;
                    }
                });
            }
            helder::right(ui, |ui| {
                match step.next() {
                    Some(next) => {
                        if helder::primary_button(ui, t, Icon::ArrowRight, NEXT).clicked() {
                            go = Some(next);
                        }
                    }
                    None => finish |= helder::primary_button(ui, t, Icon::CircleCheck, DONE).clicked(),
                }
                if let Some(back) = step.back() {
                    helder::secondary(ui, t, |ui| {
                        if ui
                            .add(egui::Button::image_and_text(
                                Icon::ArrowLeft.image(helder::ICON, t.text),
                                RichText::new(BACK).size(14.0).color(t.text),
                            ))
                            .clicked()
                        {
                            go = Some(back);
                        }
                    });
                }
            });
        });
    });
    finish |= closed;
    if finish {
        app.finish_welcome(ctx);
    } else if let Some(next) = go {
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
