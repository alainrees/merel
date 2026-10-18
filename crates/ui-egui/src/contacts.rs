//! The contacts module: a searchable list, a contact card, and the form to add or change one.

use egui::{Align2, Color32, CornerRadius, FontId, Margin, RichText, Sense, Stroke, Ui, vec2};
use serde_json::json;

use mailcraft_engine::CommandError;
use mailcraft_model::Contact;

use crate::{ComposeKind, MailApp, Tokens};

/// What the contacts module remembers between frames.
#[derive(Default)]
pub struct State {
    pub query: String,
    pub editor: Option<Editor>,
}

/// The form's fields as text; addresses and phone numbers one per line or separated by `;`.
#[derive(Default)]
pub struct Editor {
    pub id: Option<String>,
    /// Where a new contact goes: an account id, or empty for this computer.
    pub account_id: String,
    pub name: String,
    pub emails: String,
    pub phones: String,
    pub company: String,
    pub notes: String,
    pub error: Option<String>,
}

fn split(s: &str) -> Vec<String> {
    s.split([';', ',', '\n']).map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

impl Editor {
    pub fn from_contact(c: &Contact) -> Self {
        Editor {
            id: Some(c.id.clone()),
            account_id: c.account_id.clone().unwrap_or_default(),
            name: c.name.clone(),
            emails: c.emails.join("\n"),
            phones: c.phones.join("\n"),
            company: c.company.clone(),
            notes: c.notes.clone(),
            error: None,
        }
    }

    /// The `contact.save` parameters.
    pub fn params(&self) -> serde_json::Value {
        let mut p = json!({
            "name": self.name,
            "emails": split(&self.emails),
            "phones": split(&self.phones),
            "company": self.company,
            "notes": self.notes,
        });
        if let Some(id) = &self.id {
            p["id"] = json!(id);
        }
        if !self.account_id.is_empty() {
            p["account_id"] = json!(self.account_id);
        }
        p
    }
}

/// A row to pick where a new item goes, when there is a choice: this computer or an account.
pub(crate) fn store_picker(ui: &mut Ui, t: &Tokens, session: &mailcraft_engine::Session, label_width: f32, account_id: &mut String) {
    let accounts: Vec<(String, String)> = session.item_accounts().iter().map(|a| (a.id.clone(), a.name.clone())).collect();
    if accounts.is_empty() {
        return;
    }
    let name = accounts.iter().find(|(id, _)| id == account_id).map(|(_, n)| n.as_str()).unwrap_or("Op deze computer");
    ui.horizontal(|ui| {
        ui.add_sized([label_width, 22.0], egui::Label::new(RichText::new("Opslaan in").color(t.text)));
        egui::ComboBox::from_id_salt(("store-picker", label_width as i32)).selected_text(name).width(320.0).show_ui(ui, |ui| {
            ui.selectable_value(account_id, String::new(), "Op deze computer");
            for (id, n) in &accounts {
                ui.selectable_value(account_id, id.clone(), n);
            }
        });
    });
}

/// The name of the account an item lives in, for a card.
pub(crate) fn store_name(session: &mailcraft_engine::Session, account_id: Option<&str>) -> String {
    account_id.and_then(|id| session.accounts.iter().find(|a| a.id == id)).map(|a| a.name.clone()).unwrap_or_else(|| "Op deze computer".into())
}

impl MailApp {
    pub fn new_contact(&mut self) {
        self.contacts.editor = Some(Editor::default());
    }

    pub fn edit_selected_contact(&mut self) {
        if let Some(c) = self.engine.session.selected_contact() {
            self.contacts.editor = Some(Editor::from_contact(c));
        }
    }

    /// Opens a new message to the selected contact's first address.
    pub fn mail_selected_contact(&mut self) {
        let Some(address) = self.engine.session.selected_contact().and_then(|c| c.addresses().into_iter().next()) else { return };
        self.open_compose(ComposeKind::New);
        if let Some(c) = self.compose.as_mut() {
            c.to = format!("{} <{}>", address.display(), address.email());
        }
    }
}

fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("name") => "Vul een naam of een e-mailadres in.".into(),
        CommandError::Param("emails") => "Een van de e-mailadressen is niet geldig.".into(),
        CommandError::Store(m) => format!("Opslaan is mislukt: {m}"),
        CommandError::NotFound => "Dit contact bestaat niet meer.".into(),
        e => e.to_string(),
    }
}

fn list_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("contacts-list")
        .default_size(280.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(Margin::symmetric(10, 10)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut app.contacts.query).hint_text("Zoeken in contacten").desired_width(f32::INFINITY));
            ui.add_space(8.0);
            let selected = app.engine.session.selected_contact.clone();
            let list: Vec<Contact> = app.engine.session.contacts_matching(&app.contacts.query).into_iter().cloned().collect();
            if list.is_empty() {
                ui.label(
                    RichText::new(if app.contacts.query.is_empty() { "Nog geen contacten." } else { "Geen contact gevonden." }).color(t.text_dim),
                );
            }
            let mut pick = None;
            let mut initial = None;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for c in &list {
                    let letter = c.initial();
                    if initial != Some(letter) {
                        initial = Some(letter);
                        ui.add_space(4.0);
                        ui.label(RichText::new(letter.to_string()).size(12.0).strong().color(t.accent_text));
                    }
                    let on = selected.as_deref() == Some(c.id.as_str());
                    let r = ui.add_sized(
                        [ui.available_width(), 44.0],
                        egui::Button::new("").fill(if on { t.checked } else { Color32::TRANSPARENT }).frame_when_inactive(on),
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &c.name));
                    let rect = r.rect.shrink2(vec2(8.0, 4.0));
                    let p = ui.painter();
                    let badge = egui::Rect::from_min_size(rect.left_top(), vec2(34.0, 34.0)).translate(vec2(0.0, 1.0));
                    p.circle_filled(badge.center(), 17.0, t.accent);
                    p.text(badge.center(), Align2::CENTER_CENTER, letter.to_string(), FontId::proportional(15.0), t.on_accent);
                    let x = rect.left() + 44.0;
                    p.text(egui::pos2(x, rect.top() + 2.0), Align2::LEFT_TOP, &c.name, FontId::proportional(13.5), t.text);
                    let sub = c.emails.first().cloned().unwrap_or_else(|| c.company.clone());
                    p.text(egui::pos2(x, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, sub, FontId::proportional(11.5), t.text_dim);
                    if r.clicked() {
                        pick = Some(c.id.clone());
                    }
                }
            });
            if let Some(id) = pick {
                app.run("contact.select", json!({ "id": id }));
                app.contacts.editor = None;
            }
        });
}

fn row(ui: &mut Ui, t: &Tokens, label: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.add_sized([90.0, 20.0], egui::Label::new(RichText::new(label).color(t.text_dim)));
        ui.label(RichText::new(value).color(t.text));
    });
}

fn card(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(c) = app.engine.session.selected_contact().cloned() else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Kies een contact, of maak een nieuw contact.").color(t.text_dim).size(16.0));
        });
        return;
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 28.0, t.accent);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, c.initial().to_string(), FontId::proportional(24.0), t.on_accent);
        ui.vertical(|ui| {
            ui.label(RichText::new(&c.name).size(22.0).strong().color(t.text));
            if !c.company.is_empty() {
                ui.label(RichText::new(&c.company).color(t.text_dim));
            }
        });
    });
    ui.add_space(14.0);
    let mut mail = false;
    for (i, e) in c.emails.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized([90.0, 20.0], egui::Label::new(RichText::new(if i == 0 { "E-mail" } else { "" }).color(t.text_dim)));
            if ui.add(egui::Button::new(RichText::new(e).color(t.accent_text)).frame(false)).on_hover_text("Nieuw bericht aan dit adres").clicked() {
                mail = true;
            }
        });
    }
    for (i, p) in c.phones.iter().enumerate() {
        row(ui, t, if i == 0 { "Telefoon" } else { "" }, p);
    }
    if !app.engine.session.item_accounts.is_empty() {
        row(ui, t, "Opgeslagen in", &store_name(&app.engine.session, c.account_id.as_deref()));
    }
    if !c.notes.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Notities").color(t.text_dim));
        ui.label(RichText::new(&c.notes).color(t.text));
    }
    ui.add_space(14.0);
    if ui.add(egui::Button::new("Bewerken").min_size(vec2(90.0, 26.0))).clicked() {
        app.edit_selected_contact();
    }
    if mail {
        app.mail_selected_contact();
    }
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, rows: usize) {
    ui.horizontal(|ui| {
        ui.add_sized([90.0, 22.0], egui::Label::new(RichText::new(label).color(t.text)));
        if rows > 1 {
            ui.add(egui::TextEdit::multiline(value).desired_width(320.0).desired_rows(rows));
        } else {
            ui.add(egui::TextEdit::singleline(value).desired_width(320.0));
        }
    });
}

fn form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let session = &app.engine.session;
    let Some(editor) = app.contacts.editor.as_mut() else { return };
    ui.label(RichText::new(if editor.id.is_some() { "Contact bewerken" } else { "Nieuw contact" }).size(18.0).strong().color(t.text));
    ui.add_space(8.0);
    if editor.id.is_none() {
        store_picker(ui, t, session, 90.0, &mut editor.account_id);
    }
    field(ui, t, "Naam", &mut editor.name, 1);
    field(ui, t, "E-mail", &mut editor.emails, 2);
    field(ui, t, "Telefoon", &mut editor.phones, 2);
    field(ui, t, "Bedrijf", &mut editor.company, 1);
    field(ui, t, "Notities", &mut editor.notes, 4);
    ui.label(RichText::new("Meer adressen of nummers: elk op een eigen regel.").size(11.5).color(t.text_dim));
    if let Some(e) = &editor.error {
        ui.label(RichText::new(e).color(t.flag));
    }
    ui.add_space(8.0);
    let mut save = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        save = ui.add(egui::Button::new(RichText::new("Opslaan").color(t.on_accent)).fill(t.accent).min_size(vec2(90.0, 26.0))).clicked();
        cancel = ui.add(egui::Button::new("Annuleren").min_size(vec2(90.0, 26.0))).clicked();
    });
    if save {
        match app.engine.run("contact.save", &editor.params()) {
            Ok(()) => cancel = true,
            Err(e) => editor.error = Some(describe(&e)),
        }
    }
    if cancel {
        app.contacts.editor = None;
    }
}

/// The contacts module's panes; the ribbon is drawn by the caller.
pub fn show(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    list_pane(app, ui, t);
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading).inner_margin(Margin::same(24))).show(ui, |ui| {
        if app.contacts.editor.is_some() {
            form(app, ui, t);
        } else {
            card(app, ui, t);
        }
    });
}

/// The ribbon for the contacts module.
pub fn ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let has = app.engine.session.selected_contact.is_some();
    let has_mail = app.engine.session.selected_contact().is_some_and(|c| !c.emails.is_empty());
    if crate::ribbon_action(ui, t, "👤", "Nieuw contact", true) {
        app.new_contact();
    }
    ui.separator();
    if crate::ribbon_action(ui, t, "✎", "Bewerken", has) {
        app.edit_selected_contact();
    }
    if crate::ribbon_action(ui, t, "✉", "E-mail sturen", has_mail) {
        app.mail_selected_contact();
    }
    ui.separator();
    if crate::ribbon_action(ui, t, "🗑", "Verwijderen", has) {
        app.run("contact.delete", serde_json::Value::Null);
        app.contacts.editor = None;
    }
    if crate::ribbon_action(ui, t, "⟳", "Vernieuwen", !app.engine.session.item_accounts.is_empty()) {
        app.run("sync.refresh", serde_json::Value::Null);
    }
}

/// Buttons under an address field with the addresses that complete what is being typed;
/// clicking one replaces the part after the last separator.
pub fn complete_addresses(ui: &mut Ui, t: &Tokens, session: &mailcraft_engine::Session, value: &mut String) {
    let (head, tail) = match value.rfind([';', ',']) {
        Some(i) => (&value[..=i], value[i + 1..].trim_start()),
        None => ("", value.trim_start()),
    };
    if tail.is_empty() {
        return;
    }
    let suggestions = session.suggest_addresses(tail, 5);
    if suggestions.is_empty() {
        return;
    }
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(84.0);
        for a in &suggestions {
            let label = if a.display() == a.email() { a.email().to_string() } else { format!("{} <{}>", a.display(), a.email()) };
            let b = egui::Button::new(RichText::new(&label).size(12.0).color(t.text)).fill(t.hover).corner_radius(CornerRadius::same(10));
            if ui.add(b).clicked() {
                picked = Some(label);
            }
        }
    });
    if let Some(p) = picked {
        let sep = if head.is_empty() { "" } else { " " };
        *value = format!("{head}{sep}{p}; ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_splits_lists_and_keeps_the_id() {
        let c = Contact {
            id: "c1".into(),
            name: "Sanne".into(),
            emails: vec!["a@b.nl".into(), "c@d.nl".into()],
            phones: vec!["06".into()],
            company: String::new(),
            notes: String::new(),
            account_id: None,
        };
        let e = Editor::from_contact(&c);
        assert_eq!(e.emails, "a@b.nl\nc@d.nl");
        let p = e.params();
        assert_eq!(p["id"], "c1");
        assert_eq!(p["emails"].as_array().unwrap().len(), 2);
        assert_eq!(split("a@b.nl; c@d.nl,  ,e@f.nl\n"), vec!["a@b.nl", "c@d.nl", "e@f.nl"]);
    }
}
