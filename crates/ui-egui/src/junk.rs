//! The "Junk Email" window: the blocked and the safe senders, to add to or clear. The
//! message's right-click menu ("Afzender blokkeren", "Geen ongewenst") and the strip above a
//! message in the Junk folder feed the same lists. Under them: the level, and the learning
//! filter that learns from those choices.

use crate::i18n::tr;
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
    Learning(bool),
    Unlearn,
}

/// The name and the explanation of a level, as the window shows them.
pub fn level_text(level: JunkLevel) -> (String, String) {
    let (name, explanation) = match level {
        JunkLevel::Standard => ("junk-level-standard", "junk-level-standard-hint"),
        JunkLevel::High => ("junk-level-high", "junk-level-high-hint"),
        JunkLevel::SafeOnly => ("junk-level-safe-only", "junk-level-safe-only-hint"),
    };
    (tr(name), tr(explanation))
}

fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("entry") => tr("junk-entry-rule"),
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
                    if helder::icon_button(ui, t, Icon::X, &tr("junk-remove")).clicked() {
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
    let (learned_junk, learned_good) = (app.engine.session.learned.junk, app.engine.session.learned.good);
    let mut action = None;
    let mut close = false;
    if helder::popup(ctx, t, "junk", &tr("file-junk"), 520.0, |ui| {
        ui.add(egui::Label::new(RichText::new(tr("junk-intro")).size(13.0).color(t.text_dim)).wrap());
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let r = helder::underline_field(ui, t, &mut entry, &tr("junk-entry-hint"), 270.0, false);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.add(helder::primary_text(t, &tr("junk-block"))).clicked() || enter {
                action = Some(Action::Block);
            }
            if ui.button(tr("junk-trust")).clicked() {
                action = Some(Action::Trust);
            }
        });
        if let Some(e) = &error {
            ui.label(RichText::new(e).size(12.0).color(t.flag));
        }
        ui.add_space(12.0);
        ui.columns(2, |cols| {
            list(&mut cols[0], t, Icon::ShieldAlert, &tr("junk-blocked"), &lists.blocked, &tr("junk-blocked-none"), &mut action);
            list(&mut cols[1], t, Icon::ShieldCheck, &tr("junk-safe"), &lists.safe, &tr("junk-safe-none"), &mut action);
        });
        ui.add_space(16.0);
        ui.label(RichText::new(crate::i18n::tr("junk-level")).size(16.0).strong().color(t.text));
        ui.add_space(4.0);
        for level in JunkLevel::ALL {
            let (name, explanation) = level_text(level);
            if helder::choice(ui, t, &name, &explanation, lists.level == level).clicked() && lists.level != level {
                action = Some(Action::Level(level));
            }
            ui.add_space(2.0);
        }
        ui.add_space(16.0);
        ui.label(RichText::new(tr("junk-learning")).size(16.0).strong().color(t.text));
        ui.add_space(4.0);
        let mut on = !lists.learning_off;
        if helder::toggle(ui, t, &mut on, &tr("junk-learning-on")).changed() {
            action = Some(Action::Learning(on));
        }
        ui.add(egui::Label::new(RichText::new(tr("junk-learning-hint")).size(12.0).color(t.text_dim)).wrap());
        let counts = [("junk", learned_junk.into()), ("good", learned_good.into()), ("min", merel_model::MIN_TRAINED.into())];
        let ready = learned_junk >= merel_model::MIN_TRAINED && learned_good >= merel_model::MIN_TRAINED;
        let status = crate::i18n::tr_args(if ready { "junk-learning-status" } else { "junk-learning-waiting" }, &counts);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(status).size(12.0).color(t.text_dim)).wrap());
            if learned_junk + learned_good > 0 {
                helder::right(ui, |ui| {
                    if helder::add_link(ui, t, &tr("junk-learning-forget")).clicked() {
                        action = Some(Action::Unlearn);
                    }
                });
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            helder::right(ui, |ui| {
                if ui.button(crate::i18n::tr("close")).clicked() {
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
        Some(Action::Learning(on)) => app.run("junk.learning", json!({ "on": on })),
        Some(Action::Unlearn) => app.run("junk.unlearn", serde_json::Value::Null),
        None => {}
    }
    if close {
        app.junk = None;
    } else if let Some(opts) = app.junk.as_mut() {
        opts.entry = entry;
        opts.error = error;
    }
}
