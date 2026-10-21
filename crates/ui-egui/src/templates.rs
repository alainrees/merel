//! Text templates, as Outlook's "Mijn sjablonen": the "Sjablonen" menu in the compose window
//! drops a saved piece of text into the message, above the signature and the quoted text, and
//! the window behind "Sjablonen beheren…" makes and changes them.

use crate::i18n::tr;
use egui::{RichText, Ui, vec2};
use serde_json::json;

use merel_engine::CommandError;
use merel_model::Template;

use crate::{MailApp, Tokens};

pub fn menu_label() -> String {
    tr("templates-menu")
}
pub fn save_as() -> String {
    tr("template-save-as")
}
pub fn manage() -> String {
    tr("templates-manage")
}
pub fn heading() -> String {
    tr("templates-heading")
}
pub fn new() -> String {
    tr("template-new")
}
pub fn none_yet() -> String {
    tr("templates-none")
}

/// What the window remembers between frames.
#[derive(Default)]
pub struct State {
    pub form: Option<Form>,
}

/// The template being made or changed, as the person edits it.
#[derive(Clone, Debug, Default)]
pub struct Form {
    pub id: Option<String>,
    pub name: String,
    pub text: String,
    pub error: Option<String>,
}

impl Form {
    pub fn from_template(t: &Template) -> Self {
        Form { id: Some(t.id.clone()), name: t.name.clone(), text: t.text.clone(), error: None }
    }

    fn params(&self) -> serde_json::Value {
        let mut p = json!({ "name": self.name, "text": self.text });
        if let Some(id) = &self.id {
            p["id"] = json!(id);
        }
        p
    }
}

/// Where the person's own text ends in a compose body: the byte offset of the signature
/// separator, the forwarded-message line, or the reply header above the first quoted line; the
/// whole length when there is none of those.
pub fn own_text_end(body: &str) -> usize {
    let mut offset = 0;
    let mut previous: Option<(usize, bool)> = None; // start of the line before, and whether it has text
    for line in body.split_inclusive('\n') {
        let l = line.trim_end_matches(['\n', '\r']);
        if l == "-- " || l.starts_with("----------") {
            return offset;
        }
        if l.starts_with('>') {
            return match previous {
                Some((start, true)) => start,
                _ => offset,
            };
        }
        previous = Some((offset, !l.trim().is_empty()));
        offset += line.len();
    }
    body.len()
}

/// The body with `text` dropped in after the person's own text, above the signature and the
/// quote, with a blank line around it.
pub fn insert(body: &str, text: &str) -> String {
    let end = own_text_end(body);
    let own = body[..end].trim_end();
    let rest = &body[end..];
    let mut out = String::new();
    if !own.is_empty() {
        out.push_str(own);
        out.push_str("\n\n");
    }
    out.push_str(text.trim_end());
    if rest.is_empty() {
        return out;
    }
    if !rest.starts_with('\n') {
        out.push_str("\n\n");
    }
    out.push_str(rest);
    out
}

/// The person's own text in a compose body, without the signature and the quote.
pub fn own_text(body: &str) -> &str {
    body[..own_text_end(body)].trim()
}

fn save_error(e: &CommandError) -> String {
    match e {
        CommandError::Param("name") => tr("template-name-rule"),
        CommandError::Param("text") => tr("template-text-rule"),
        CommandError::Store(m) => crate::i18n::tr_args("template-save-failed", &[("error", m.to_string().into())]),
        e => e.to_string(),
    }
}

/// The "Sjablonen" menu in the compose window: a template to drop in, saving the text written
/// so far, and the window.
pub fn menu(ui: &mut Ui, t: &crate::Tokens, templates: &[&Template]) -> Option<Pick> {
    let mut pick = None;
    crate::helder::menu_button(ui, t, Some(crate::helder::Icon::FileText), RichText::new(menu_label()).size(14.0).color(t.text), |ui| {
        for t in templates {
            if ui.button(&t.name).on_hover_text(t.first_line()).clicked() {
                pick = Some(Pick::Insert(t.text.clone()));
                ui.close();
            }
        }
        if templates.is_empty() {
            ui.label(RichText::new(none_yet()).size(12.0));
        }
        ui.separator();
        if ui.button(save_as()).clicked() {
            pick = Some(Pick::SaveAs);
            ui.close();
        }
        if ui.button(manage()).clicked() {
            pick = Some(Pick::Manage);
            ui.close();
        }
    })
    .on_hover_text(tr("templates-menu-hint"));
    pick
}

/// What the menu asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Insert(String),
    SaveAs,
    Manage,
}

enum Action {
    New,
    Edit(String),
    Delete(String),
}

/// The "Sjablonen" window, when it is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    if app.templates.is_none() {
        return;
    }
    let templates: Vec<Template> = app.engine.session.templates_sorted().into_iter().cloned().collect();
    let mut action: Option<Action> = None;
    let mut save = false;
    let mut cancel = false;
    let mut close = false;
    let mut form_state = app.templates.as_mut().and_then(|s| s.form.take());
    egui::Window::new(heading()).collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
        ui.label(RichText::new(tr("templates-intro")).size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        if let Some(form) = form_state.as_mut() {
            ui.label(RichText::new(if form.id.is_some() { tr("template-edit") } else { tr("template-new") }).strong().color(t.text));
            ui.horizontal(|ui| {
                ui.add_sized([60.0, 24.0], egui::Label::new(RichText::new(tr("name")).color(t.text_dim)));
                ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(360.0));
            });
            ui.horizontal(|ui| {
                ui.add_sized([60.0, 24.0], egui::Label::new(RichText::new(tr("text")).color(t.text_dim)));
                ui.add(egui::TextEdit::multiline(&mut form.text).desired_width(360.0).desired_rows(8));
            });
            if let Some(e) = &form.error {
                ui.label(RichText::new(e).color(t.flag));
            }
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(tr("save")).min_size(vec2(90.0, 26.0))).clicked() {
                    save = true;
                }
                if ui.button(tr("cancel")).clicked() {
                    cancel = true;
                }
            });
            return;
        }
        if ui.button(new()).clicked() {
            action = Some(Action::New);
        }
        ui.add_space(6.0);
        if templates.is_empty() {
            ui.label(RichText::new(none_yet()).color(t.text_dim));
        }
        for tpl in &templates {
            egui::Frame::new().fill(t.list).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new(&tpl.name).strong().color(t.text));
                ui.label(RichText::new(tpl.first_line()).size(12.0).color(t.text_dim));
                ui.horizontal(|ui| {
                    if ui.button(tr("edit")).clicked() {
                        action = Some(Action::Edit(tpl.id.clone()));
                    }
                    if ui.button(tr("template-delete")).clicked() {
                        action = Some(Action::Delete(tpl.id.clone()));
                    }
                });
            });
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
        if ui.button(tr("close")).clicked() {
            close = true;
        }
    });
    if save && let Some(form) = form_state.as_mut() {
        match app.engine.run("template.save", &form.params()) {
            Ok(()) => form_state = None,
            Err(e) => form.error = Some(save_error(&e)),
        }
    }
    if cancel {
        form_state = None;
    }
    match action {
        Some(Action::New) => form_state = Some(Form::default()),
        Some(Action::Edit(id)) => form_state = app.engine.session.template(&id).map(Form::from_template),
        Some(Action::Delete(id)) => {
            if let Err(e) = app.engine.run("template.delete", &json!({ "id": id })) {
                app.notice = Some(crate::Notice::error(e.to_string()));
            }
        }
        None => {}
    }
    if close {
        app.templates = None;
    } else if let Some(state) = app.templates.as_mut() {
        state.form = form_state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_goes_above_the_signature_and_the_quote() {
        assert_eq!(insert("", "Dank je."), "Dank je.");
        assert_eq!(insert("\n\n-- \nAlain", "Dank je."), "Dank je.\n\n-- \nAlain");
        assert_eq!(insert("Hoi Sanne,\n\n-- \nAlain", "Dank je."), "Hoi Sanne,\n\nDank je.\n\n-- \nAlain");
        let reply = "\n\nOp 1 oktober schreef Sanne:\n> tien uur?";
        assert_eq!(insert(reply, "Prima."), "Prima.\n\nOp 1 oktober schreef Sanne:\n> tien uur?");
        let forward = "Kijk.\n\n---------- Doorgestuurd bericht ----------\nVan: x";
        assert_eq!(insert(forward, "Groet"), "Kijk.\n\nGroet\n\n---------- Doorgestuurd bericht ----------\nVan: x");
        assert_eq!(insert("Alleen tekst", "Meer"), "Alleen tekst\n\nMeer");
        assert_eq!(own_text("Hoi Sanne,\n\n-- \nAlain"), "Hoi Sanne,");
        assert_eq!(own_text(reply), "");
    }
}
