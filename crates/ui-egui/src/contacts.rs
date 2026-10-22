//! The contacts module: a searchable list, a contact card, and the form to add or change one.

use crate::i18n::tr;
use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Ui, vec2};
use serde_json::json;

use merel_engine::CommandError;
use merel_model::{Contact, PhoneKind, Photo, Place, PlaceKind};

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
    /// The tab of the card on show.
    pub tab: CardTab,
}

/// The tabs of a contact card, as Outlook has them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CardTab {
    #[default]
    Contact,
    Organisation,
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
    pub phones: Vec<(PhoneKind, String)>,
    pub company: String,
    pub job_title: String,
    pub department: String,
    pub office: String,
    pub places: Vec<Place>,
    pub website: String,
    /// The day of birth as dag-maand-jaar, as typed or picked.
    pub birthday: String,
    pub photo: Option<Photo>,
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
            job_title: String::new(),
            department: String::new(),
            office: String::new(),
            places: Vec::new(),
            website: String::new(),
            birthday: String::new(),
            photo: None,
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
            phones: c.phones.iter().map(|p| (p.kind, p.number.clone())).collect(),
            company: c.company.clone(),
            job_title: c.job_title.clone(),
            department: c.department.clone(),
            office: c.office.clone(),
            places: c.places.clone(),
            website: c.website.clone(),
            birthday: c
                .birthday
                .as_deref()
                .and_then(|b| chrono::NaiveDate::parse_from_str(b, "%Y-%m-%d").ok())
                .map(|d| d.format("%d-%m-%Y").to_string())
                .unwrap_or_default(),
            photo: c.photo.clone(),
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

    /// The day of birth as `YYYY-MM-DD`: empty for none, `None` for what is not a day.
    pub fn birthday(&self) -> Option<String> {
        let b = self.birthday.trim();
        if b.is_empty() {
            return Some(String::new());
        }
        crate::calendar::parse_date(b).map(|d| d.format("%Y-%m-%d").to_string())
    }

    /// The `contact.save` parameters.
    pub fn params(&self) -> serde_json::Value {
        use base64::Engine as _;
        let phones: Vec<serde_json::Value> =
            self.phones.iter().filter(|(_, n)| !n.trim().is_empty()).map(|(kind, n)| json!({ "kind": kind, "number": n.trim() })).collect();
        let mut p = json!({
            "name": self.name(),
            "emails": split(&self.emails),
            "phones": phones,
            "company": self.company,
            "job_title": self.job_title,
            "department": self.department,
            "office": self.office,
            "website": self.website,
            "birthday": self.birthday().unwrap_or_default(),
            "places": self.places,
            "photo": self.photo.as_ref().map(|p| base64::engine::general_purpose::STANDARD.encode(&p.0)),
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
    let local = crate::calendars::local();
    let name = accounts.iter().find(|(id, _)| id == account_id).map(|(_, n)| n.as_str()).unwrap_or(&local);
    helder::icon_row(ui, t, Icon::Folder, |ui| {
        helder::select(ui, t, name, 260.0, |ui| {
            if ui.selectable_label(account_id.is_empty(), &local).clicked() {
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
        .on_hover_text(tr("contacts-store-in"));
    });
}

/// The name of the account an item lives in, for a card.
pub(crate) fn store_name(session: &merel_engine::Session, account_id: Option<&str>) -> String {
    account_id
        .and_then(|id| session.accounts.iter().find(|a| a.id == id))
        .map(|a| a.address.email().to_string())
        .unwrap_or_else(crate::calendars::local)
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
        let Some(c) = self.engine.session.selected_contact().cloned() else { return };
        if let Some(e) = c.emails.first() {
            self.mail_contact_at(&c, e);
        }
    }

    /// Opens a new message to `email`, named after the contact.
    fn mail_contact_at(&mut self, contact: &Contact, email: &str) {
        let Some(address) = contact.addresses().into_iter().find(|a| a.email().eq_ignore_ascii_case(email)) else { return };
        self.open_compose(ComposeKind::New);
        if let Some(c) = self.compose.as_mut() {
            c.to = format!("{} <{}>", address.display(), address.email());
        }
    }
}

fn describe(e: &CommandError) -> String {
    match e {
        CommandError::Param("name") => tr("contacts-need-name"),
        CommandError::Param("emails") => tr("contacts-bad-email"),
        CommandError::Store(m) => crate::i18n::tr_args("save-failed-because", &[("error", m.as_str().into())]),
        CommandError::NotFound => tr("contacts-gone"),
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
            crate::helder::search_field(ui, t, &mut app.contacts.query, egui::Id::new(SEARCH_BOX), &tr("contacts-search"), width);
            ui.add_space(8.0);
            let selected = app.engine.session.selected_contact.clone();
            let list: Vec<Contact> = app.engine.session.contacts_matching(&app.contacts.query).into_iter().cloned().collect();
            let groups: Vec<merel_model::ContactGroup> = app.engine.session.groups_sorted().into_iter().cloned().collect();
            let selected_group = app.contacts.group.clone();
            let mut group_pick = None;
            if list.is_empty() {
                ui.label(
                    RichText::new(if app.contacts.query.is_empty() { tr("contacts-none-yet") } else { tr("contacts-none-found") }).color(t.text_dim),
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
                    let badge = egui::Rect::from_min_size(rect.left_top(), vec2(34.0, 34.0)).translate(vec2(0.0, 1.0));
                    helder::portrait(ui, badge.center(), 17.0, &c.name, c.emails.first().map(String::as_str).unwrap_or(""), c.photo.as_ref());
                    let p = ui.painter();
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

/// What a field on the card does when its value is clicked.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Act {
    Mail(String),
    Chat(String),
    Call(String),
    Web(String),
    Notes,
}

/// One field of the card: an icon, what it is, and its value in lines.
struct Field {
    icon: Icon,
    label: String,
    lines: Vec<String>,
    act: Option<Act>,
}

impl Field {
    fn text(icon: Icon, label: String, value: &str) -> Option<Field> {
        let lines: Vec<String> = value.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
        (!lines.is_empty()).then_some(Field { icon, label, lines, act: None })
    }

    fn link(icon: Icon, label: String, value: &str, act: Act) -> Option<Field> {
        Field::text(icon, label, value).map(|f| Field { act: Some(act), ..f })
    }
}

pub fn phone_label(kind: PhoneKind) -> String {
    match kind {
        PhoneKind::Mobile => tr("contacts-mobile"),
        PhoneKind::Work => tr("contacts-phone-work"),
        PhoneKind::Home => tr("contacts-phone-home"),
        PhoneKind::Other => tr("contacts-phone"),
    }
}

pub fn place_label(kind: PlaceKind) -> String {
    match kind {
        PlaceKind::Work => tr("contacts-address-work"),
        PlaceKind::Home => tr("contacts-address-home"),
        PlaceKind::Other => tr("contacts-address-other"),
    }
}

/// The number as a `tel:` link, which the computer hands to its phone app: digits and a
/// leading plus only.
fn tel_link(number: &str) -> Option<String> {
    let digits: String = number.chars().enumerate().filter(|(i, c)| c.is_ascii_digit() || (*i == 0 && *c == '+')).map(|(_, c)| c).collect();
    (digits.trim_start_matches('+').len() >= 3).then(|| format!("tel:{digits}"))
}

fn phone_fields(c: &Contact, kinds: &[PhoneKind]) -> Vec<Field> {
    let mut sorted: Vec<&merel_model::Phone> = c.phones.iter().filter(|p| kinds.contains(&p.kind)).collect();
    sorted.sort_by_key(|p| kinds.iter().position(|k| *k == p.kind));
    sorted.into_iter().filter_map(|p| Field::link(Icon::Phone, phone_label(p.kind), &p.number, Act::Call(p.number.clone()))).collect()
}

fn place_fields(c: &Contact, kinds: &[PlaceKind]) -> Vec<Field> {
    kinds
        .iter()
        .flat_map(|k| c.places.iter().filter(move |p| p.kind == *k))
        .filter_map(|p| Field::text(Icon::MapPin, place_label(p.kind), &p.lines().join("\n")))
        .collect()
}

fn birthday_text(c: &Contact) -> String {
    c.birthday
        .as_deref()
        .and_then(|b| chrono::NaiveDate::parse_from_str(b, "%Y-%m-%d").ok())
        .map(|d| {
            use chrono::Datelike;
            format!("{} {} {}", d.day(), crate::calendar::month_name(d.month()), d.year())
        })
        .unwrap_or_default()
}

/// The fields of the tab Contactpersoon, in Outlook's order.
fn contact_fields(app: &MailApp, c: &Contact) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    for e in &c.emails {
        out.extend(Field::link(Icon::Mail, tr("contacts-email-label"), e, Act::Mail(e.clone())));
    }
    if app.can_chat()
        && let Some(e) = c.emails.first()
    {
        out.extend(Field::link(Icon::MessagesSquare, tr("contacts-chat"), e, Act::Chat(e.clone())));
    }
    out.extend(phone_fields(c, &[PhoneKind::Mobile, PhoneKind::Work, PhoneKind::Home, PhoneKind::Other]));
    out.extend(Field::text(Icon::Building, tr("contacts-company"), &c.company));
    out.extend(Field::text(Icon::User, tr("contacts-job-title"), &c.job_title));
    out.extend(place_fields(c, &[PlaceKind::Work, PlaceKind::Home, PlaceKind::Other]));
    if let Some(link) = c.website_link() {
        out.extend(Field::link(Icon::Globe, tr("contacts-website"), &c.website, Act::Web(link)));
    }
    out.extend(Field::text(Icon::Calendar, tr("contacts-birthday"), &birthday_text(c)));
    if !app.engine.session.item_accounts.is_empty() {
        out.extend(Field::text(Icon::Folder, tr("contacts-stored-in"), &store_name(&app.engine.session, c.account_id.as_deref())));
    }
    out
}

/// The fields of the tab Organisatie.
fn organisation_fields(c: &Contact) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    out.extend(Field::text(Icon::Building, tr("contacts-company"), &c.company));
    out.extend(Field::text(Icon::Users, tr("contacts-department"), &c.department));
    out.extend(Field::text(Icon::User, tr("contacts-job-title"), &c.job_title));
    out.extend(Field::text(Icon::MapPin, tr("contacts-office"), &c.office));
    out.extend(phone_fields(c, &[PhoneKind::Work]));
    out.extend(place_fields(c, &[PlaceKind::Work]));
    if let Some(link) = c.website_link() {
        out.extend(Field::link(Icon::Globe, tr("contacts-website"), &c.website, Act::Web(link)));
    }
    out
}

/// The fields side by side in up to three columns, as Outlook lays out a contact; returns
/// what was clicked.
fn field_grid(ui: &mut Ui, t: &Tokens, fields: &[Field]) -> Option<Act> {
    const GAP: f32 = 24.0;
    let width = ui.available_width();
    let columns = ((width + GAP) / (220.0 + GAP)).floor().clamp(1.0, 3.0) as usize;
    let cell = (width - GAP * (columns - 1) as f32) / columns as f32;
    let mut act = None;
    for row in fields.chunks(columns) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for f in row {
                ui.allocate_ui_with_layout(vec2(cell, 0.0), Layout::left_to_right(Align::Min), |ui| {
                    ui.set_width(cell);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::hover());
                    f.icon.image(20.0, t.text_dim).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(20.0, 20.0)));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(&f.label).size(12.5).color(t.text_dim));
                        for line in &f.lines {
                            match &f.act {
                                Some(a) => {
                                    let r = ui
                                        .add(egui::Label::new(RichText::new(line).size(14.0).color(t.accent_text)).sense(Sense::click()).wrap())
                                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                                    let r = match a {
                                        Act::Mail(_) => r.on_hover_text(tr("contacts-mail-hint")),
                                        Act::Chat(_) => r.on_hover_text(tr("contacts-chat-hint")),
                                        Act::Call(_) => r.on_hover_text(tr("contacts-call-hint")),
                                        Act::Web(_) => r.on_hover_text(tr("contacts-website-hint")),
                                        Act::Notes => r,
                                    };
                                    if r.clicked() {
                                        act = Some(a.clone());
                                    }
                                }
                                None => {
                                    ui.add(egui::Label::new(RichText::new(line).size(14.0).color(t.text)).wrap());
                                }
                            }
                        }
                    });
                });
            }
        });
        ui.add_space(14.0);
    }
    act
}

/// A square button with only an icon on a grey face, as under the name on Outlook's card.
fn quick_button(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    let color = if enabled { t.text } else { t.text_disabled };
    let r = ui.add_enabled(
        enabled,
        egui::Button::image(icon.image(18.0, color))
            .fill(t.window)
            .stroke(egui::Stroke::NONE)
            .corner_radius(helder::CONTROL_RADIUS)
            .min_size(vec2(40.0, 40.0)),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    r.on_hover_text(label)
}

fn card(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(c) = app.engine.session.selected_contact().cloned() else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(tr("contacts-pick")).color(t.text_dim).size(16.0));
        });
        return;
    };
    let mut export = false;
    helder::quiet(ui, t, |ui| {
        ui.horizontal(|ui| {
            if helder::tool_button(ui, t, Icon::Pencil, &tr("edit")).clicked() {
                app.edit_selected_contact();
            }
            if helder::tool_button(ui, t, Icon::Download, &crate::vcard::export_label()).on_hover_text(tr("contacts-export-hint")).clicked() {
                export = true;
            }
        });
    });
    ui.add_space(12.0);
    let email = c.emails.first().cloned();
    let mobile = c.phone(PhoneKind::Mobile).or_else(|| c.phones.first().map(|p| p.number.as_str())).map(str::to_string);
    let mut act = None;
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
        helder::portrait(ui, rect.center(), 46.0, &c.name, email.as_deref().unwrap_or(""), c.photo.as_ref());
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(&c.name).size(22.0).strong().color(t.text));
            let sub = [c.job_title.as_str(), c.company.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            if !sub.is_empty() {
                ui.label(RichText::new(sub).size(14.0).color(t.text_dim));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if quick_button(ui, t, Icon::Mail, &tr("contacts-quick-mail"), email.is_some()).clicked() {
                    act = email.clone().map(Act::Mail);
                }
                if app.can_chat() && quick_button(ui, t, Icon::MessagesSquare, &tr("contacts-quick-chat"), email.is_some()).clicked() {
                    act = email.clone().map(Act::Chat);
                }
                if quick_button(ui, t, Icon::Phone, &tr("contacts-quick-call"), mobile.as_deref().and_then(tel_link).is_some()).clicked() {
                    act = mobile.clone().map(Act::Call);
                }
            });
        });
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (tab, label) in [(CardTab::Contact, tr("contacts-tab-contact")), (CardTab::Organisation, tr("contacts-tab-organisation"))] {
            if helder::tab(ui, t, &label, app.contacts.tab == tab).clicked() {
                app.contacts.tab = tab;
            }
        }
    });
    ui.painter().hline(ui.min_rect().x_range(), ui.cursor().top() - ui.spacing().item_spacing.y / 2.0, egui::Stroke::new(1.0, t.card_border));
    ui.add_space(16.0);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        match app.contacts.tab {
            CardTab::Contact => {
                ui.label(RichText::new(tr("contacts-details")).size(16.0).strong().color(t.text));
                ui.add_space(10.0);
                let fields = contact_fields(app, &c);
                if let Some(a) = field_grid(ui, t, &fields) {
                    act = Some(a);
                }
                // The notes span the card, with a way to add some when there are none.
                let notes = match Field::text(Icon::Notes, tr("contacts-notes"), &c.notes) {
                    Some(f) => f,
                    None => Field { icon: Icon::Notes, label: tr("contacts-notes"), lines: vec![tr("contacts-add-notes")], act: Some(Act::Notes) },
                };
                ui.scope(|ui| {
                    if let Some(a) = field_grid(ui, t, std::slice::from_ref(&notes)) {
                        act = Some(a);
                    }
                });
            }
            CardTab::Organisation => {
                let fields = organisation_fields(&c);
                if fields.is_empty() {
                    ui.label(RichText::new(tr("contacts-no-organisation")).size(14.0).color(t.text_dim));
                } else if let Some(a) = field_grid(ui, t, &fields) {
                    act = Some(a);
                }
            }
        }
    });
    if export {
        app.export_selected_contact();
    }
    match act {
        Some(Act::Mail(e)) => app.mail_contact_at(&c, &e),
        Some(Act::Chat(e)) => app.chat_with(&e),
        Some(Act::Call(n)) => {
            if let Some(link) = tel_link(&n) {
                let _ = open::that_detached(link);
            }
        }
        Some(Act::Web(link)) => {
            let _ = open::that_detached(link);
        }
        Some(Act::Notes) => app.edit_selected_contact(),
        None => {}
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
                if many
                    && helder::quiet(ui, t, |ui| {
                        helder::icon_button(ui, t, Icon::X, &crate::i18n::tr_args("contacts-remove-row", &[("what", hint.into())]))
                    })
                    .clicked()
                {
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

/// A choice of kinds shown as a small menu, such as Mobiel or Werk for a phone number.
fn kind_select<K: Copy + PartialEq>(ui: &mut Ui, t: &Tokens, kind: &mut K, kinds: &[(K, String)], id: &str) {
    let shown = kinds.iter().find(|(k, _)| k == kind).map(|(_, n)| n.clone()).unwrap_or_default();
    ui.push_id(id, |ui| {
        helder::select(ui, t, &shown, 110.0, |ui| {
            for (k, name) in kinds {
                if ui.selectable_label(k == kind, name).clicked() {
                    *kind = *k;
                    ui.close();
                }
            }
        });
    });
}

fn phone_kinds() -> [(PhoneKind, String); 4] {
    [
        (PhoneKind::Mobile, tr("contacts-mobile")),
        (PhoneKind::Work, tr("contacts-kind-work")),
        (PhoneKind::Home, tr("contacts-kind-home")),
        (PhoneKind::Other, tr("contacts-kind-other")),
    ]
}

fn place_kinds() -> [(PlaceKind, String); 3] {
    [(PlaceKind::Work, tr("contacts-kind-work")), (PlaceKind::Home, tr("contacts-kind-home")), (PlaceKind::Other, tr("contacts-kind-other"))]
}

/// The phone numbers, each with its kind; a new row is a mobile number first, as Outlook
/// offers, then a work number.
fn phone_rows(ui: &mut Ui, t: &Tokens, phones: &mut Vec<(PhoneKind, String)>, width: f32) {
    if phones.is_empty() {
        phones.push((PhoneKind::Mobile, String::new()));
    }
    let kinds = phone_kinds();
    let hint = tr("contacts-phone-number");
    let many = phones.len() > 1;
    let mut remove = None;
    for (i, (kind, number)) in phones.iter_mut().enumerate() {
        ui.push_id(("phone", i), |ui| {
            helder::icon_row(ui, t, Icon::Phone, |ui| {
                kind_select(ui, t, kind, &kinds, "kind");
                helder::underline_field(ui, t, number, &hint, (width - 44.0 - 122.0).max(120.0), false);
                if many
                    && helder::quiet(ui, t, |ui| {
                        helder::icon_button(ui, t, Icon::X, &crate::i18n::tr_args("contacts-remove-row", &[("what", hint.as_str().into())]))
                    })
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        phones.remove(i);
    }
    ui.horizontal(|ui| {
        helder::indent(ui);
        if helder::quiet(ui, t, |ui| helder::add_link(ui, t, &tr("contacts-add-phone"))).clicked() {
            let kind = if phones.iter().any(|(k, _)| *k == PhoneKind::Mobile) { PhoneKind::Work } else { PhoneKind::Mobile };
            phones.push((kind, String::new()));
        }
    });
}

/// The postal addresses, each as street, postal code and city, region and country.
fn place_rows(ui: &mut Ui, t: &Tokens, places: &mut Vec<Place>, width: f32) {
    let kinds = place_kinds();
    let field = width - 44.0;
    let mut remove = None;
    for (i, place) in places.iter_mut().enumerate() {
        ui.push_id(("place", i), |ui| {
            helder::icon_row(ui, t, Icon::MapPin, |ui| {
                kind_select(ui, t, &mut place.kind, &kinds, "kind");
                ui.add_space((field - 110.0 - 40.0 - 12.0).max(0.0));
                let what = place_label(place.kind);
                if helder::quiet(ui, t, |ui| {
                    helder::icon_button(ui, t, Icon::X, &crate::i18n::tr_args("contacts-remove-row", &[("what", what.into())]))
                })
                .clicked()
                {
                    remove = Some(i);
                }
            });
            ui.horizontal(|ui| {
                helder::indent(ui);
                helder::underline_field(ui, t, &mut place.street, &tr("contacts-street"), field, false);
            });
            ui.horizontal(|ui| {
                helder::indent(ui);
                helder::underline_field(ui, t, &mut place.postal_code, &tr("contacts-postal-code"), 110.0, false);
                helder::underline_field(ui, t, &mut place.city, &tr("contacts-city"), (field - 122.0).max(100.0), false);
            });
            ui.horizontal(|ui| {
                helder::indent(ui);
                let half = (field - 12.0) / 2.0;
                helder::underline_field(ui, t, &mut place.region, &tr("contacts-region"), half, false);
                helder::underline_field(ui, t, &mut place.country, &tr("contacts-country"), half, false);
            });
            ui.add_space(6.0);
        });
    }
    if let Some(i) = remove {
        places.remove(i);
    }
    ui.horizontal(|ui| {
        helder::indent(ui);
        if helder::quiet(ui, t, |ui| helder::add_link(ui, t, &tr("contacts-add-address"))).clicked() {
            let used: Vec<PlaceKind> = places.iter().map(|p| p.kind).collect();
            let kind = [PlaceKind::Work, PlaceKind::Home, PlaceKind::Other].into_iter().find(|k| !used.contains(k)).unwrap_or(PlaceKind::Other);
            places.push(Place { kind, ..Place::default() });
        }
    });
}

/// The most a picture file may be to be read.
const MAX_PICTURE_FILE: u64 = 20 * 1024 * 1024;
/// A chosen picture is cut square and made this many pixels wide, as servers keep them.
const PICTURE_SIDE: u32 = 256;

/// A picture chosen on the computer as a contact photo: read with limits, cut square from
/// the middle, made [`PICTURE_SIDE`] pixels and stored as JPEG. Decoding and encoding it
/// anew also leaves out whatever else the file carried, such as where it was taken.
pub fn picture_from_file(path: &std::path::Path) -> Result<Photo, String> {
    let bad = || tr("contacts-photo-unreadable");
    let meta = std::fs::metadata(path).map_err(|_| bad())?;
    if !meta.is_file() || meta.len() > MAX_PICTURE_FILE {
        return Err(tr("contacts-photo-too-large"));
    }
    picture_from_bytes(&std::fs::read(path).map_err(|_| bad())?).ok_or_else(bad)
}

pub fn picture_from_bytes(bytes: &[u8]) -> Option<Photo> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let side = img.width().min(img.height());
    let square = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side);
    let small = if side > PICTURE_SIDE { square.resize_exact(PICTURE_SIDE, PICTURE_SIDE, image::imageops::FilterType::Lanczos3) } else { square };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&small.to_rgb8()).ok()?;
    Photo::new(out)
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
            save = helder::primary_button(ui, t, Icon::Save, &tr("save")).clicked();
            helder::bar_separator(ui, t);
            cancel = helder::tool_button(ui, t, Icon::X, &tr("cancel")).clicked();
        });
    });
    ui.add_space(12.0);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let width = ui.available_width().min(620.0);
        ui.horizontal_top(|ui| {
            // The contact's picture: its initials once there is a name, else a person.
            ui.vertical(|ui| {
                ui.set_width(110.0);
                let (rect, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
                let name = editor.name();
                let email = editor.emails.first().cloned().unwrap_or_default();
                if name.is_empty() && email.trim().is_empty() && editor.photo.is_none() {
                    ui.painter().circle(rect.center(), 46.0, t.window, egui::Stroke::new(1.5, t.control_border));
                    Icon::User.image(44.0, t.text_dim).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(44.0, 44.0)));
                } else {
                    helder::portrait(ui, rect.center(), 46.0, &name, &email, editor.photo.as_ref());
                }
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                let link = |ui: &mut Ui, text: String| {
                    ui.add_sized([96.0, 22.0], egui::Button::new(RichText::new(text).size(13.0).color(t.accent_text)).frame(false))
                };
                if link(ui, if editor.photo.is_some() { tr("contacts-photo-change") } else { tr("contacts-photo-add") }).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title(tr("contacts-photo-pick"))
                        .add_filter(tr("contacts-photo-files"), &["jpg", "jpeg", "png", "gif", "webp", "bmp"])
                        .pick_file()
                {
                    match picture_from_file(&path) {
                        Ok(photo) => editor.photo = Some(photo),
                        Err(e) => editor.error = Some(e),
                    }
                }
                if editor.photo.is_some() && link(ui, tr("contacts-photo-remove")).clicked() {
                    editor.photo = None;
                }
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                let field = (width - 130.0).max(200.0);
                ui.label(RichText::new(if editor.id.is_some() { tr("contacts-edit-title") } else { tr("new-contact") }).size(13.0).color(t.text_dim));
                ui.spacing_mut().item_spacing.y = 10.0;
                helder::underline_field(ui, t, &mut editor.first, &tr("contacts-first-name"), field, true);
                helder::underline_field(ui, t, &mut editor.last, &tr("contacts-last-name"), field, true);
            });
        });
        ui.add_space(4.0);
        if editor.id.is_none() {
            store_picker(ui, t, session, &mut editor.account_id);
        }
        helder::rule(ui, t);
        heading(ui, t, &tr("contacts-emails"));
        rows(ui, t, Icon::Mail, &mut editor.emails, &tr("contacts-email"), &tr("contacts-add-email"), width);
        helder::rule(ui, t);
        heading(ui, t, &tr("contacts-phones"));
        phone_rows(ui, t, &mut editor.phones, width);
        helder::rule(ui, t);
        heading(ui, t, &tr("contacts-work"));
        for (icon, value, hint) in [
            (Icon::Building, &mut editor.company, tr("contacts-company")),
            (Icon::User, &mut editor.job_title, tr("contacts-job-title")),
            (Icon::Users, &mut editor.department, tr("contacts-department")),
            (Icon::MapPin, &mut editor.office, tr("contacts-office")),
        ] {
            helder::icon_row(ui, t, icon, |ui| helder::underline_field(ui, t, value, &hint, width - 44.0, false));
        }
        helder::rule(ui, t);
        heading(ui, t, &tr("contacts-addresses"));
        place_rows(ui, t, &mut editor.places, width);
        helder::rule(ui, t);
        heading(ui, t, &tr("contacts-more"));
        helder::icon_row(ui, t, Icon::Globe, |ui| helder::underline_field(ui, t, &mut editor.website, &tr("contacts-website"), width - 44.0, false));
        helder::icon_row(ui, t, Icon::Calendar, |ui| {
            crate::date_picker::field(ui, t, &mut editor.birthday, &tr("contacts-birthday-field"), 200.0, &tr("contacts-birthday-pick"));
        });
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
    if save && editor.birthday().is_none() {
        editor.error = Some(tr("contacts-bad-birthday"));
        save = false;
    }
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
            ..Contact::default()
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
