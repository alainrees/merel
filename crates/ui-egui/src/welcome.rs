//! The welcome at the first start: a short window in four steps (what Merel is, an
//! account, updates, light or dark) that the person can leave at any step. It comes once per
//! computer: [`ViewPrefs::welcomed`](crate::view::ViewPrefs) is set when it closes, however it
//! closes. Every choice in it can be changed later in the same place as always (Bestand, Over
//! Merel, Beeld).

use crate::i18n::tr;
use egui::RichText;

use crate::helder::{self, Icon};
use crate::theme::Tokens;
use crate::view::ThemeChoice;
use crate::{MailApp, Tab};

/// The window's title and buttons, which tests and screen readers find them by.
pub fn window_label() -> String {
    tr("welcome-title")
}
pub fn next_label() -> String {
    tr("welcome-next")
}
pub fn back_label() -> String {
    tr("welcome-back")
}
pub fn done() -> String {
    tr("welcome-done")
}
pub fn skip() -> String {
    tr("welcome-skip")
}
pub fn add_account() -> String {
    tr("account-add-title")
}
pub fn updates_on() -> String {
    tr("welcome-updates-on")
}

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

    fn title(self) -> String {
        tr(match self {
            Step::Hello => "welcome-hello-title",
            Step::Account => "welcome-account-title",
            Step::Updates => "welcome-updates-title",
            Step::Look => "welcome-look-title",
        })
    }
}

/// The welcome window, while it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(step) = app.welcome else { return };
    let mut go: Option<Step> = None;
    let mut finish = false;
    let closed = helder::dialog(ctx, t, &window_label(), 480.0, |ui| {
        helder::card_frame(t).inner_margin(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if step == Step::Hello {
                    helder::app_badge(ui)
                } else {
                    helder::badge(ui, t, step.icon())
                }
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
                    helder::paragraph(ui, t, &tr("welcome-hello-1"));
                    ui.add_space(6.0);
                    helder::paragraph(ui, t, &tr("welcome-hello-2"));
                }
                Step::Account => {
                    helder::paragraph(ui, t, &tr("welcome-account-text"));
                    ui.add_space(10.0);
                    helder::secondary(ui, t, |ui| {
                        if ui
                            .add(egui::Button::image_and_text(
                                Icon::UserPlus.image(helder::ICON, t.text),
                                RichText::new(add_account()).size(14.0).color(t.text),
                            ))
                            .clicked()
                        {
                            app.tab = Tab::File;
                            finish = true;
                        }
                    });
                }
                Step::Updates => {
                    helder::paragraph(ui, t, &tr("welcome-updates-text"));
                    ui.add_space(10.0);
                    let mut on = app.view.update_check == Some(true);
                    helder::toggle(ui, t, &mut on, &updates_on());
                    app.view.update_check = Some(on);
                    ui.add(egui::Label::new(RichText::new(tr("welcome-updates-self")).size(12.0).color(t.text_dim)).wrap());
                }
                Step::Look => {
                    helder::paragraph(ui, t, &tr("welcome-look-text"));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        helder::quiet(ui, t, |ui| {
                            for (choice, icon) in
                                [(ThemeChoice::System, Icon::Monitor), (ThemeChoice::Light, Icon::Sun), (ThemeChoice::Dark, Icon::Moon)]
                            {
                                if helder::toggle_button(ui, t, icon, &choice.name(), app.view.theme == choice).on_hover_text(choice.hint()).clicked()
                                {
                                    app.view.theme = choice;
                                    ctx.set_theme(choice.preference());
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        helder::quiet(ui, t, |ui| {
                            for choice in crate::i18n::LanguageChoice::ALL {
                                if helder::toggle_button(ui, t, Icon::Globe, &choice.name(), app.view.language == choice)
                                    .on_hover_text(choice.hint())
                                    .clicked()
                                {
                                    app.view.language = choice;
                                    app.apply_language();
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
                    if helder::text_button(ui, t, &skip()).on_hover_text(tr("welcome-skip-hint")).clicked() {
                        finish = true;
                    }
                });
            }
            helder::right(ui, |ui| {
                match step.next() {
                    Some(next) => {
                        if helder::primary_button(ui, t, Icon::ArrowRight, &next_label()).clicked() {
                            go = Some(next);
                        }
                    }
                    None => finish |= helder::primary_button(ui, t, Icon::CircleCheck, &done()).clicked(),
                }
                if let Some(back) = step.back() {
                    helder::secondary(ui, t, |ui| {
                        if ui
                            .add(egui::Button::image_and_text(
                                Icon::ArrowLeft.image(helder::ICON, t.text),
                                RichText::new(back_label()).size(14.0).color(t.text),
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
