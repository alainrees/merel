//! The "Ongewenste e-mail" window: the blocked and the safe senders, to add to or clear. The
//! message's right-click menu ("Afzender blokkeren", "Geen ongewenst") and the strip above a
//! message in the Junk folder feed the same lists.

use egui::{RichText, Ui};
use serde_json::json;

use merel_engine::CommandError;
use merel_model::JunkLevel;

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

fn list(ui: &mut Ui, t: &Tokens, title: &str, entries: &[String], empty: &str, action: &mut Option<Action>) {
    ui.label(RichText::new(title).strong().color(t.text));
    ui.add_space(4.0);
    if entries.is_empty() {
        ui.label(RichText::new(empty).size(12.0).color(t.text_dim));
    }
    for e in entries {
        ui.horizontal(|ui| {
            if ui.small_button("Weghalen").on_hover_text("Van de lijst halen").clicked() {
                *action = Some(Action::Forget(e.clone()));
            }
            ui.label(RichText::new(e).color(t.text));
        });
    }
}

pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(opts) = app.junk.as_ref() else { return };
    let (mut entry, mut error) = (opts.entry.clone(), opts.error.clone());
    let lists = app.engine.session.junk.clone();
    let mut action = None;
    let mut close = false;
    egui::Window::new("Ongewenste e-mail").collapsible(false).resizable(false).default_width(520.0).show(ctx, |ui| {
        ui.label(
            RichText::new(
                "Mail van een geblokkeerde afzender of een geblokkeerd domein gaat bij aankomst meteen naar Ongewenste e-mail. \
                 Veilige afzenders en je contacten komen daar nooit in. Je mailprovider filtert daarnaast zelf; wat die als ongewenst ziet, staat ook in die map.",
            )
            .size(12.0)
            .color(t.text_dim),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut entry).hint_text("naam@voorbeeld.nl of voorbeeld.nl").desired_width(260.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Blokkeren").clicked() || enter {
                action = Some(Action::Block);
            }
            if ui.button("Vertrouwen").clicked() {
                action = Some(Action::Trust);
            }
        });
        if let Some(e) = &error {
            ui.label(RichText::new(e).size(12.0).color(t.flag));
        }
        ui.add_space(10.0);
        ui.columns(2, |cols| {
            list(&mut cols[0], t, "Geblokkeerde afzenders", &lists.blocked, "Nog niemand geblokkeerd.", &mut action);
            list(&mut cols[1], t, "Veilige afzenders", &lists.safe, "Je contacten zijn altijd veilig.", &mut action);
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Niveau").strong().color(t.text));
        ui.add_space(4.0);
        for level in JunkLevel::ALL {
            let (name, explanation) = level_text(level);
            if ui.radio(lists.level == level, name).on_hover_text(explanation).clicked() && lists.level != level {
                action = Some(Action::Level(level));
            }
            ui.label(RichText::new(explanation).size(12.0).color(t.text_dim));
        }
        ui.add_space(10.0);
        if ui.button("Sluiten").clicked() {
            close = true;
        }
    });
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
