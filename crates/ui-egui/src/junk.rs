//! The "Ongewenste e-mail" window: the blocked and the safe senders, to add to or clear. The
//! message's right-click menu ("Afzender blokkeren", "Geen ongewenst") and the strip above a
//! message in the Junk folder feed the same lists.

use egui::{RichText, Ui};
use serde_json::json;

use merel_engine::CommandError;
use merel_model::JunkLevel;

use crate::helder::{self, Icon};
use crate::{MailApp, Tokens};

/// What the window holds besides the lists in the session.
#[derive(Default)]
pub struct Options {
    pub entry: String,
    pub error: Option<String>,
}

enum Action {
    Block,
    Trust,
    Forget(String),
    Level(JunkLevel),
}

/// The name and the explanation of a level, as the window shows them.
pub fn level_text(level: JunkLevel) -> (&'static str, &'static str) {
    match level {
        JunkLevel::Standard => ("Standaard", "Alleen mail van geblokkeerde afzenders gaat naar Ongewenste e-mail; je mailprovider filtert de rest."),
        JunkLevel::High => (
            "Hoog",
            "Ook mail van een afzender met kenmerken van phishing: de naam van een contact met een ander adres, of een domein dat op een bekend domein lijkt.",
        ),
        JunkLevel::SafeOnly => (
            "Alleen veilige lijsten",
            "Alles wat niet van een contact, een veilige afzender of jezelf komt, gaat naar Ongewenste e-mail. Kijk daar dan regelmatig.",
        ),
    }
}

fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("entry") => "Vul een e-mailadres of een domein in, zoals naam@voorbeeld.nl of voorbeeld.nl.".into(),
        e => e.to_string(),
    }
}

/// One of the two lists, on a card: its title with how many there are, then each entry with a
/// button that lets it go.
fn list(ui: &mut Ui, t: &Tokens, icon: Icon, title: &str, entries: &[String], empty: &str, action: &mut Option<Action>) {
    helder::card_frame(t).fill(t.window).inner_margin(12).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add(icon.image(helder::ICON, t.accent_text));
            ui.label(RichText::new(title).size(14.0).strong().color(t.text));
            if !entries.is_empty() {
                ui.label(RichText::new(entries.len().to_string()).size(12.0).color(t.text_dim));
            }
        });
        ui.add_space(4.0);
        if entries.is_empty() {
            ui.add(egui::Label::new(RichText::new(empty).size(12.0).color(t.text_dim)).wrap());
        }
        for e in entries {
            ui.horizontal(|ui| {
                ui.label(RichText::new(e).color(t.text));
                helder::right(ui, |ui| {
                    if helder::icon_button(ui, t, Icon::X, "Weghalen").clicked() {
                        *action = Some(Action::Forget(e.clone()));
                    }
                });
            });
        }
    });
}

pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(opts) = app.junk.as_ref() else { return };
    let (mut entry, mut error) = (opts.entry.clone(), opts.error.clone());
    let lists = app.engine.session.junk.clone();
    let mut action = None;
    let mut close = false;
    if helder::popup(ctx, t, "junk", "Ongewenste e-mail", 520.0, |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Mail van een geblokkeerde afzender of een geblokkeerd domein gaat bij aankomst meteen naar Ongewenste e-mail. \
                     Veilige afzenders en je contacten komen daar nooit in. Je mailprovider filtert daarnaast zelf; wat die als ongewenst ziet, staat ook in die map.",
                )
                .size(13.0)
                .color(t.text_dim),
            )
            .wrap(),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let r = helder::underline_field(ui, t, &mut entry, "naam@voorbeeld.nl of voorbeeld.nl", 270.0, false);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.add(helder::primary_text(t, "Blokkeren")).clicked() || enter {
                action = Some(Action::Block);
            }
            if ui.button("Vertrouwen").clicked() {
                action = Some(Action::Trust);
            }
        });
        if let Some(e) = &error {
            ui.label(RichText::new(e).size(12.0).color(t.flag));
        }
        ui.add_space(12.0);
        ui.columns(2, |cols| {
            list(&mut cols[0], t, Icon::ShieldAlert, "Geblokkeerde afzenders", &lists.blocked, "Nog niemand geblokkeerd.", &mut action);
            list(&mut cols[1], t, Icon::ShieldCheck, "Veilige afzenders", &lists.safe, "Je contacten zijn altijd veilig.", &mut action);
        });
        ui.add_space(16.0);
        ui.label(RichText::new("Niveau").size(16.0).strong().color(t.text));
        ui.add_space(4.0);
        for level in JunkLevel::ALL {
            let (name, explanation) = level_text(level);
            if helder::choice(ui, t, name, explanation, lists.level == level).clicked() && lists.level != level {
                action = Some(Action::Level(level));
            }
            ui.add_space(2.0);
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            helder::right(ui, |ui| {
                if ui.button("Sluiten").clicked() {
                    close = true;
                }
            });
        });
    }) {
        close = true;
    }
    match action {
        Some(Action::Block) | Some(Action::Trust) => {
            let id = if matches!(action, Some(Action::Block)) { "junk.block" } else { "junk.trust" };
            match app.engine.run(id, &json!({ "entry": entry })) {
                Ok(()) => {
                    entry.clear();
                    error = None;
                }
                Err(e) => error = Some(describe(&e)),
            }
        }
        Some(Action::Forget(e)) => app.run("junk.forget", json!({ "entry": e })),
        Some(Action::Level(level)) => app.run("junk.level", json!({ "level": level.id() })),
        None => {}
    }
    if close {
        app.junk = None;
    } else if let Some(opts) = app.junk.as_mut() {
        opts.entry = entry;
        opts.error = error;
    }
}
