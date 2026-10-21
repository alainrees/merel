//! The contacts module: a searchable list, a contact card, and the form to add or change one.

use crate::i18n::tr;
use egui::{Align2, Color32, CornerRadius, FontId, Margin, RichText, Sense, Ui, vec2};
use serde_json::json;

use merel_engine::CommandError;
use merel_model::Contact;

use crate::helder::{self, Icon};
use crate::{ComposeKind, MailApp, Module, Tokens};

/// What the contacts module remembers between frames.
#[derive(Default)]
pub struct State {
    pub query: String,
    pub editor: Option<Editor>,
    /// The contact group whose card is shown, if any; a contact click clears it.
    pub group: Option<String>,
    pub group_editor: Option<crate::groups::Editor>,
}

/// The search box's widget id, so Ctrl+E can put the cursor there.
pub(crate) const SEARCH_BOX: &str = "contacts_search_box";

/// The form's fields: the name in two parts, as Outlook asks for it, and each address and
/// phone number on its own row.
pub struct Editor {
    pub id: Option<String>,
    /// Where a new contact goes: an account id, or empty for this computer.
    pub account_id: String,
    pub first: String,
    pub last: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub company: String,
    pub notes: String,
    /// The formatting of the notes, with the bar over them as in the message window.
    pub notes_editor: crate::rich_edit::Editor,
    pub error: Option<String>,
}

/// The notes editor's salt: a contact's form shows one at a time.
const NOTES: &str = "contact-notes";

impl Default for Editor {
    fn default() -> Self {
        Editor {
            id: None,
            account_id: String::new(),
            first: String::new(),
            last: String::new(),
            emails: Vec::new(),
            phones: Vec::new(),
            company: String::new(),
            notes: String::new(),
            notes_editor: crate::rich_edit::Editor::new(NOTES, "", None),
            error: None,
        }
    }
}

/// The parts of a row, a pasted "a@b.nl; c@d.nl" giving two.
fn split(rows: &[String]) -> Vec<String> {
    rows.iter().flat_map(|r| r.split([';', ',', '\n'])).map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

/// A full name as first name and the rest: "Sanne de Vries" is Sanne and "de Vries".
pub fn split_name(name: &str) -> (String, String) {
    match name.trim().split_once(char::is_whitespace) {
        Some((first, last)) => (first.to_string(), last.trim().to_string()),
        None => (name.trim().to_string(), String::new()),
    }
}

impl Editor {
    pub fn from_contact(c: &Contact) -> Self {
        let (first, last) = split_name(&c.name);
        Editor {
            id: Some(c.id.clone()),
            account_id: c.account_id.clone().unwrap_or_default(),
            first,
            last,
            emails: c.emails.clone(),
            phones: c.phones.clone(),
            company: c.company.clone(),
            notes: c.notes.clone(),
            notes_editor: crate::rich_edit::Editor::new(NOTES, &c.notes, c.notes_rich.clone()),
            error: None,
        }
    }

    /// A new contact with this name and address, as "Toevoegen aan contacten" starts one.
    pub fn new_for(name: &str, email: &str) -> Self {
        let (first, last) = split_name(name);
        Editor { first, last, emails: vec![email.to_string()], ..Editor::default() }
    }

    /// The name the two fields make.
    pub fn name(&self) -> String {
        [self.first.trim(), self.last.trim()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
    }

    /// The `contact.save` parameters.
    pub fn params(&self) -> serde_json::Value {
        let mut p = json!({
            "name": self.name(),
            "emails": split(&self.emails),
            "phones": split(&self.phones),
            "company": self.company,
            "notes": self.notes,
            "notes_rich": self.notes_editor.stored(&self.notes),
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

/// A row to pick where a new item goes, when there is a choice: this computer or an account,
/// by its address, since two accounts often carry the same name.
pub(crate) fn store_picker(ui: &mut Ui, t: &Tokens, session: &merel_engine::Session, account_id: &mut String) {
    let accounts: Vec<(String, String)> = session.item_accounts().iter().map(|a| (a.id.clone(), a.address.email().to_string())).collect();
    if accounts.is_empty() {
        return;
    }
    let name = accounts.iter().find(|(id, _)| id == account_id).map(|(_, n)| n.as_str()).unwrap_or("Op deze computer");
    helder::icon_row(ui, t, Icon::Folder, |ui| {
        helder::select(ui, t, name, 260.0, |ui| {
            if ui.selectable_label(account_id.is_empty(), "Op deze computer").clicked() {
                account_id.clear();
                ui.close();
            }
            for (id, n) in &accounts {
                if ui.selectable_label(account_id == id, n).clicked() {
                    *account_id = id.clone();
                    ui.close();
                }
            }
        })
        .on_hover_text("Opslaan in");
    });
}

/// The name of the account an item lives in, for a card.
pub(crate) fn store_name(session: &merel_engine::Session, account_id: Option<&str>) -> String {
    account_id
        .and_then(|id| session.accounts.iter().find(|a| a.id == id))
        .map(|a| a.address.email().to_string())
        .unwrap_or_else(|| "Op deze computer".into())
}

impl MailApp {
    /// Opens the contacts module on the contact that has `from`'s address, or on the form for
    /// a new contact named after the sender, so an address from the mail is kept in one click.
    pub fn open_or_add_sender(&mut self, from: &merel_model::Address) {
        self.module = Module::People;
        if let Some(c) = self.engine.session.contact_with_email(from.email()) {
            let id = c.id.clone();
            self.run("contact.select", json!({ "id": id }));
            self.contacts.editor = None;
            return;
        }
        let name = if from.display() == from.email() { "" } else { from.display() };
        self.contacts.editor = Some(Editor::new_for(name, from.email()));
    }

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
        .show_separator_line(false)
        .frame(crate::helder::card_frame(t).inner_margin(Margin::same(12)).outer_margin(Margin { left: 0, right: 6, top: 0, bottom: 12 }))
        .show(ui, |ui| {
            let width = ui.available_width();
            crate::helder::search_field(ui, t, &mut app.contacts.query, egui::Id::new(SEARCH_BOX), "Zoeken in contacten", width);
            ui.add_space(8.0);
            let selected = app.engine.session.selected_contact.clone();
            let list: Vec<Contact> = app.engine.session.contacts_matching(&app.contacts.query).into_iter().cloned().collect();
            let groups: Vec<merel_model::ContactGroup> = app.engine.session.groups_sorted().into_iter().cloned().collect();
            let selected_group = app.contacts.group.clone();
            let mut group_pick = None;
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
                        egui::Button::new("")
                            .fill(if on { t.checked } else { Color32::TRANSPARENT })
                            .frame_when_inactive(on)
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(helder::CONTROL_RADIUS),
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &c.name));
                    let rect = r.rect.shrink2(vec2(8.0, 4.0));
                    let p = ui.painter();
                    let badge = egui::Rect::from_min_size(rect.left_top(), vec2(34.0, 34.0)).translate(vec2(0.0, 1.0));
                    helder::avatar(p, badge.center(), 17.0, &c.name, c.emails.first().map(String::as_str).unwrap_or(""));
                    let x = rect.left() + 44.0;
                    p.text(egui::pos2(x, rect.top() + 2.0), Align2::LEFT_TOP, &c.name, FontId::proportional(13.5), t.text);
                    let sub = c.emails.first().cloned().unwrap_or_else(|| c.company.clone());
                    p.text(egui::pos2(x, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, sub, FontId::proportional(11.5), t.text_dim);
                    if r.clicked() {
                        pick = Some(c.id.clone());
                    }
                }
                group_pick = crate::groups::section(ui, t, &groups, selected_group.as_deref());
            });
            if let Some(id) = pick {
                app.run("contact.select", json!({ "id": id }));
                app.contacts.editor = None;
                app.contacts.group = None;
            }
            if let Some(id) = group_pick {
                app.contacts.group = Some(id);
                app.contacts.editor = None;
            }
        });
}

fn row(ui: &mut Ui, t: &Tokens, icon: Icon, value: &str, hint: &str) {
    if value.is_empty() {
        return;
    }
    helder::icon_row(ui, t, icon, |ui| ui.label(RichText::new(value).size(14.0).color(t.text))).on_hover_text(hint);
}

fn card(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(c) = app.engine.session.selected_contact().cloned() else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Kies een contact, of maak een nieuw contact.").color(t.text_dim).size(16.0));
        });
        return;
    };
    let mut export = false;
    helder::quiet(ui, t, |ui| {
        ui.horizontal(|ui| {
            if helder::tool_button(ui, t, Icon::Pencil, "Bewerken").clicked() {
                app.edit_selected_contact();
            }
            if helder::tool_button(ui, t, Icon::Download, crate::vcard::EXPORT)
                .on_hover_text("Bewaart dit contact als .vcf-bestand in je map Downloads, om te delen of in een ander programma te openen.")
                .clicked()
            {
                export = true;
            }
        });
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
        helder::avatar(ui.painter(), rect.center(), 34.0, &c.name, c.emails.first().map(String::as_str).unwrap_or(""));
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(&c.name).size(22.0).strong().color(t.text));
            if !c.company.is_empty() {
                ui.label(RichText::new(&c.company).size(14.0).color(t.text_dim));
            }
        });
    });
    helder::rule(ui, t);
    let mut mail = false;
    for e in &c.emails {
        helder::icon_row(ui, t, Icon::Mail, |ui| {
            if ui
                .add(egui::Button::new(RichText::new(e).size(14.0).color(t.accent_text)).frame(false))
                .on_hover_text("Nieuw bericht aan dit adres")
                .clicked()
            {
                mail = true;
            }
        });
    }
    for p in &c.phones {
        row(ui, t, Icon::Phone, p, "Telefoon");
    }
    row(ui, t, Icon::Building, &c.company, "Bedrijf");
    if !app.engine.session.item_accounts.is_empty() {
        row(ui, t, Icon::Folder, &store_name(&app.engine.session, c.account_id.as_deref()), "Opgeslagen in");
    }
    if !c.notes.is_empty() {
        helder::rule(ui, t);
        helder::icon_row(ui, t, Icon::Notes, |ui| ui.add(egui::Label::new(RichText::new(&c.notes).size(14.0).color(t.text)).wrap()));
    }
    if export {
        app.export_selected_contact();
    }
    if mail {
        app.mail_selected_contact();
    }
}

/// Rows of one kind of field, such as the e-mail addresses: each on its own line with a way to
/// take it out, then a link that adds one. Always at least one row to type in.
fn rows(ui: &mut Ui, t: &Tokens, icon: Icon, values: &mut Vec<String>, hint: &str, add: &str, width: f32) {
    if values.is_empty() {
        values.push(String::new());
    }
    let mut remove = None;
    let many = values.len() > 1;
    for (i, v) in values.iter_mut().enumerate() {
        ui.push_id((hint, i), |ui| {
            helder::icon_row(ui, t, icon, |ui| {
                helder::underline_field(ui, t, v, hint, width - 44.0, false);
                if many && helder::quiet(ui, t, |ui| helder::icon_button(ui, t, Icon::X, &format!("{hint} weghalen"))).clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        values.remove(i);
    }
    ui.horizontal(|ui| {
        helder::indent(ui);
        if helder::quiet(ui, t, |ui| helder::add_link(ui, t, add)).clicked() {
            values.push(String::new());
        }
    });
}

fn heading(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.horizontal(|ui| {
        helder::indent(ui);
        ui.label(RichText::new(text).size(15.0).strong().color(t.text));
    });
}

fn form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let session = &app.engine.session;
    let Some(editor) = app.contacts.editor.as_mut() else { return };
    let mut save = false;
    let mut cancel = false;
    helder::quiet(ui, t, |ui| {
        ui.horizontal(|ui| {
            save = helder::primary_button(ui, t, Icon::Save, "Opslaan").clicked();
            helder::bar_separator(ui, t);
            cancel = helder::tool_button(ui, t, Icon::X, "Annuleren").clicked();
        });
    });
    ui.add_space(12.0);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let width = ui.available_width().min(620.0);
        ui.horizontal_top(|ui| {
            // The contact's picture: its initials once there is a name, else a person.
            let (rect, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
            let name = editor.name();
            let email = editor.emails.first().cloned().unwrap_or_default();
            if name.is_empty() && email.trim().is_empty() {
                ui.painter().circle(rect.center(), 46.0, t.window, egui::Stroke::new(1.5, t.control_border));
                Icon::User.image(44.0, t.text_dim).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(44.0, 44.0)));
            } else {
                helder::avatar(ui.painter(), rect.center(), 46.0, &name, &email);
            }
            ui.add_space(12.0);
            ui.vertical(|ui| {
                let field = (width - 108.0).max(200.0);
                ui.label(RichText::new(if editor.id.is_some() { "Contact bewerken" } else { "Nieuw contact" }).size(13.0).color(t.text_dim));
                ui.spacing_mut().item_spacing.y = 10.0;
                helder::underline_field(ui, t, &mut editor.first, "Voornaam", field, true);
                helder::underline_field(ui, t, &mut editor.last, "Achternaam", field, true);
            });
        });
        ui.add_space(4.0);
        if editor.id.is_none() {
            store_picker(ui, t, session, &mut editor.account_id);
        }
        helder::rule(ui, t);
        heading(ui, t, "E-mailadressen");
        rows(ui, t, Icon::Mail, &mut editor.emails, "E-mailadres", "E-mail toevoegen", width);
        helder::rule(ui, t);
        heading(ui, t, "Telefoonnummers");
        rows(ui, t, Icon::Phone, &mut editor.phones, "Telefoonnummer", "Nummer toevoegen", width);
        helder::rule(ui, t);
        heading(ui, t, "Werkgegevens");
        helder::icon_row(ui, t, Icon::Building, |ui| helder::underline_field(ui, t, &mut editor.company, "Bedrijf", width - 44.0, false));
        helder::rule(ui, t);
        helder::notes_box(ui, t, &mut editor.notes_editor, &mut editor.notes, width - 44.0, 100.0);
        if let Some(e) = &editor.error {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                helder::indent(ui);
                ui.label(RichText::new(e).color(t.flag));
            });
        }
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
    let frame = crate::helder::card_frame(t).inner_margin(Margin::same(24)).outer_margin(Margin { left: 6, right: 12, top: 0, bottom: 12 });
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        if app.contacts.editor.is_some() {
            form(app, ui, t);
        } else if let Some(id) = app.contacts.group.clone() {
            crate::groups::card(app, ui, t, &id);
        } else {
            card(app, ui, t);
        }
    });
}

/// The ribbon for the contacts module.
pub fn ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let has = app.engine.session.selected_contact.is_some();
    let has_mail = app.engine.session.selected_contact().is_some_and(|c| !c.emails.is_empty());
    if crate::ribbon_action_hint(ui, t, "👤", &tr("new-contact"), true, Some(&crate::key_hint("Shift+C"))) {
        app.new_contact();
    }
    if crate::ribbon_action(ui, t, "👥", &tr("contacts-new-group"), true) {
        app.new_group();
    }
    crate::ribbon_separator(ui);
    if crate::ribbon_action(ui, t, "✎", &tr("edit"), has) {
        app.edit_selected_contact();
    }
    if crate::ribbon_action(ui, t, "✉", &tr("contacts-send-mail"), has_mail) {
        app.mail_selected_contact();
    }
    crate::ribbon_separator(ui);
    if crate::ribbon_action(ui, t, "🗑", &tr("delete"), has) {
        app.run("contact.delete", serde_json::Value::Null);
        app.contacts.editor = None;
    }
    if crate::ribbon_action(ui, t, "⟳", &tr("refresh"), !app.engine.session.item_accounts.is_empty()) {
        app.run("sync.refresh", serde_json::Value::Null);
    }
    crate::ribbon_separator(ui);
    if crate::ribbon_action(ui, t, "📂", &tr("contacts-import"), true) {
        // The computer's own dialog; it blocks until the person has chosen or cancelled.
        if let Some(paths) = rfd::FileDialog::new().set_title(tr("contacts-import-title")).add_filter("vCard", &["vcf", "vcard"]).pick_files() {
            app.import_contacts(&paths);
        }
    }
}

/// Buttons under an address field with the addresses that complete what is being typed;
/// clicking one replaces the part after the last separator.
pub fn complete_addresses(ui: &mut Ui, t: &Tokens, session: &merel_engine::Session, value: &mut String) {
    let (head, tail) = match value.rfind([';', ',']) {
        Some(i) => (&value[..=i], value[i + 1..].trim_start()),
        None => ("", value.trim_start()),
    };
    if tail.is_empty() {
        return;
    }
    // A contact group's name offers all its members at once, as Outlook expands a group.
    let groups: Vec<(String, String)> =
        session.groups_matching(tail).into_iter().take(3).map(|g| (crate::groups::label(g), g.address_line())).collect();
    let suggestions = session.suggest_addresses(tail, 5);
    if suggestions.is_empty() && groups.is_empty() {
        return;
    }
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(84.0);
        for (label, line) in &groups {
            let b = egui::Button::new(RichText::new(label).size(12.0).color(t.text)).fill(t.checked).corner_radius(CornerRadius::same(10));
            if ui.add(b).on_hover_text(line).clicked() {
                picked = Some(line.clone());
            }
        }
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
            notes_rich: None,
        };
        let e = Editor::from_contact(&c);
        assert_eq!(e.emails, ["a@b.nl", "c@d.nl"]);
        let p = e.params();
        assert_eq!(p["id"], "c1");
        assert_eq!(p["emails"].as_array().unwrap().len(), 2);
        assert_eq!(split(&["a@b.nl; c@d.nl,  ".into(), "e@f.nl\n".into(), String::new()]), vec!["a@b.nl", "c@d.nl", "e@f.nl"]);
        assert_eq!(split_name("Sanne de Vries"), ("Sanne".to_string(), "de Vries".to_string()));
        assert_eq!(Editor::new_for("Nieuwsbrief", "n@b.nl").name(), "Nieuwsbrief");
    }
}
