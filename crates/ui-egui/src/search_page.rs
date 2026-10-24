//! Bestand > Zoeken: the search index on this computer. Whether Merel keeps one, how far back
//! it fetches the text of messages, how far it got, and a way to throw it away.

use crate::i18n::{tr, tr_args};
use egui::{RichText, Ui};
use merel_engine::indexing::IndexSettings;

use crate::{MailApp, Notice, Tokens, helder};

pub fn page_label() -> String {
    tr("search-page")
}
pub fn enabled_label() -> String {
    tr("search-index-on")
}
pub fn clear_label() -> String {
    tr("search-index-clear")
}

/// The choices for how far back, in years; `None` is all mail.
const SPANS: [Option<u32>; 4] = [None, Some(5), Some(2), Some(1)];

fn span_label(years: Option<u32>) -> String {
    match years {
        None => tr("search-index-all"),
        Some(1) => tr("search-index-one-year"),
        Some(y) => tr_args("search-index-years", &[("years", y.into())]),
    }
}

/// A size in bytes as people read it.
fn size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if (bytes as f64) < MB { format!("{} kB", (bytes / 1024).max(1)) } else { format!("{:.0} MB", bytes as f64 / MB) }
}

pub fn page(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let hint = |ui: &mut Ui, text: &str| {
        ui.horizontal(|ui| {
            ui.add_space(44.0);
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(t.text_dim)).wrap());
        });
    };
    let section = |ui: &mut Ui, text: &str| {
        ui.add_space(10.0);
        ui.label(RichText::new(text).size(14.0).strong().color(t.text));
    };
    ui.label(RichText::new(page_label()).size(helder::TITLE_TEXT).strong().color(t.text));
    ui.add(egui::Label::new(RichText::new(tr("search-page-intro")).size(13.0).color(t.text_dim)).wrap());
    if app.engine.session.is_sample() {
        ui.add_space(10.0);
        ui.label(RichText::new(tr("search-index-sample")).size(13.0).color(t.text_dim));
        return;
    }
    let before = app.engine.index_settings().clone();
    let mut settings = before.clone();
    section(ui, &tr("search-index-section"));
    helder::toggle(ui, t, &mut settings.enabled, &enabled_label());
    hint(ui, &tr("search-index-on-hint"));
    if settings.enabled && app.engine.index_on() {
        let p = app.engine.index_progress();
        let line = tr_args("search-index-progress", &[("text", p.with_text.into()), ("messages", p.messages.into()), ("size", size(p.size).into())]);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(44.0);
            ui.label(RichText::new(line).size(13.0).color(t.text));
        });
        if p.busy {
            hint(ui, &tr("search-index-busy"));
            // The numbers move while the index fills.
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
    let mut clear = false;
    if settings.enabled {
        section(ui, &tr("search-index-span"));
        hint(ui, &tr("search-index-span-hint"));
        ui.add_space(4.0);
        for years in SPANS {
            if helder::choice(ui, t, &span_label(years), "", settings.years == years).clicked() {
                settings.years = years;
            }
        }
        ui.add_space(10.0);
        clear = ui.button(clear_label()).on_hover_text(tr("search-index-clear-hint")).clicked();
    }
    if settings != before {
        save(app, settings);
    }
    if clear {
        app.notice = Some(match app.engine.clear_index() {
            Ok(()) => Notice::success(tr("search-index-cleared")),
            Err(e) => Notice::error(e),
        });
    }
}

fn save(app: &mut MailApp, settings: IndexSettings) {
    if let Err(e) = app.engine.set_index_settings(settings) {
        app.notice = Some(Notice::error(e));
    }
}
