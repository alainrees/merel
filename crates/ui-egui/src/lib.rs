//! Mailcraft's front end: an Outlook-style window drawn with egui in the ArtCraft look.
//!
//! Layout, top to bottom: the tab strip and ribbon, then the module rail (Mail, Calendar,
//! People, Tasks), the folder pane, the message list and the reading pane, then the status bar.
//! Every button runs a command id through [`mailcraft_engine::Session::run`]; nothing here
//! changes the session directly.

mod backstage;
pub mod calendar;
mod compose;
pub mod contacts;
mod dates;
pub mod tasks;
mod theme;

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};

use mailcraft_engine::{AccountStatus, Engine};
use mailcraft_model::FolderKind;

pub use backstage::AccountForm;
pub use compose::{Compose, Kind as ComposeKind};

pub use theme::{APP_COLOR, Tokens};

/// The four Outlook modules, in the order they are being built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Module {
    Mail,
    Calendar,
    People,
    Tasks,
}

impl Module {
    const ALL: [Module; 4] = [Module::Mail, Module::Calendar, Module::People, Module::Tasks];

    fn label(self) -> &'static str {
        match self {
            Module::Mail => "E-mail",
            Module::Calendar => "Agenda",
            Module::People => "Contacten",
            Module::Tasks => "Taken",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Module::Mail => "✉",
            Module::Calendar => "📅",
            Module::People => "👥",
            Module::Tasks => "✔",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    File,
    Home,
    SendReceive,
    View,
    Help,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::File, Tab::Home, Tab::SendReceive, Tab::View, Tab::Help];

    fn label(self) -> &'static str {
        match self {
            Tab::File => "Bestand",
            Tab::Home => "Start",
            Tab::SendReceive => "Verzenden/ontvangen",
            Tab::View => "Beeld",
            Tab::Help => "Help",
        }
    }
}

pub struct MailApp {
    pub engine: Engine,
    pub module: Module,
    tab: Tab,
    form: AccountForm,
    pub compose: Option<Compose>,
    pub calendar: calendar::State,
    pub contacts: contacts::State,
    pub tasks: tasks::State,
    /// The last command error or note, shown in the status bar until the next action.
    notice: Option<String>,
}

impl MailApp {
    pub fn new(engine: Engine) -> Self {
        MailApp {
            engine,
            module: Module::Mail,
            tab: Tab::Home,
            form: AccountForm::default(),
            compose: None,
            calendar: calendar::State::default(),
            contacts: contacts::State::default(),
            tasks: tasks::State::default(),
            notice: None,
        }
    }

    /// Runs a command; an error is shown, never raised.
    fn run(&mut self, id: &str, params: Value) {
        self.notice = match self.engine.run(id, &params) {
            Ok(()) => None,
            Err(e) => {
                log::warn!("{id}: {e}");
                Some(match e {
                    mailcraft_engine::CommandError::NoSelection => "Selecteer eerst een bericht.".to_string(),
                    mailcraft_engine::CommandError::NoFolder(_) => "Dit account heeft die map niet.".to_string(),
                    e => e.to_string(),
                })
            }
        };
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let t = Tokens::for_dark(ui.visuals().dark_mode);
        self.engine.poll();
        if self.engine.session.has_pending() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        ribbon(self, ui, &t);
        status_bar(self, ui, &t);
        let ctx = ui.ctx().clone();
        compose::show(self, &ctx, &t);
        calendar::window(self, &ctx, &t);
        if self.tab == Tab::File {
            backstage::show(self, ui, &t);
            return;
        }
        rail(self, ui, &t);
        if self.module == Module::Mail {
            folder_pane(self, ui, &t);
            message_list(self, ui, &t);
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading).inner_margin(Margin::same(20))).show(ui, |ui| {
                reading_pane(self, ui, &t);
            });
        } else if self.module == Module::Calendar {
            calendar::show(self, ui, &t);
        } else if self.module == Module::People {
            contacts::show(self, ui, &t);
        } else {
            tasks::show(self, ui, &t);
        }
    }
}

fn ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::top("tabs").frame(egui::Frame::NONE.fill(t.tab_strip).inner_margin(Margin::symmetric(8, 4))).show(ui, |ui| {
        ui.horizontal(|ui| {
            for tab in Tab::ALL {
                let on = app.tab == tab;
                let text = RichText::new(tab.label()).color(if on { t.accent_text } else { t.text });
                let r = ui.add(egui::Button::new(text).frame(false).min_size(vec2(0.0, 26.0)));
                if on {
                    let rect = r.rect;
                    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, t.accent));
                }
                if r.clicked() {
                    app.tab = tab;
                }
            }
        });
    });
    egui::Panel::top("ribbon")
        .frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(Margin::symmetric(10, 8)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal(|ui| match app.tab {
                Tab::Home if app.module == Module::Calendar => calendar_ribbon(app, ui, t),
                Tab::Home if app.module == Module::People => contacts::ribbon(app, ui, t),
                Tab::Home if app.module == Module::Tasks => tasks::ribbon(app, ui, t),
                Tab::Home => {
                    let has = app.engine.session.selected_message.is_some();
                    let mut open: Option<ComposeKind> = None;
                    if ribbon_action(ui, t, "✉", "Nieuwe e-mail", true) {
                        open = Some(ComposeKind::New);
                    }
                    ui.separator();
                    if ribbon_action(ui, t, "↩", "Beantwoorden", has) {
                        open = Some(ComposeKind::Reply);
                    }
                    if ribbon_action(ui, t, "↞", "Allen beantwoorden", has) {
                        open = Some(ComposeKind::ReplyAll);
                    }
                    if ribbon_action(ui, t, "↪", "Doorsturen", has) {
                        open = Some(ComposeKind::Forward);
                    }
                    if let Some(kind) = open {
                        app.open_compose(kind);
                    }
                    ui.separator();
                    ribbon_button(app, ui, t, "🗑", "Verwijderen", Some("message.delete"), has);
                    ribbon_button(app, ui, t, "🗄", "Archiveren", Some("message.archive"), has);
                    ribbon_button(app, ui, t, "⚠", "Ongewenst", Some("message.junk"), has);
                    ui.separator();
                    ribbon_button(app, ui, t, "◉", "Gelezen", Some("message.mark_read"), has);
                    ribbon_button(app, ui, t, "○", "Ongelezen", Some("message.mark_unread"), has);
                    ribbon_button(app, ui, t, "⚑", "Vlag", Some("message.flag"), has);
                }
                Tab::SendReceive => {
                    let can = !app.engine.session.is_sample() && app.engine.session.selected_folder.is_some();
                    ribbon_button(app, ui, t, "⟳", "Vernieuwen", Some("folder.refresh"), can);
                }
                _ => {
                    ui.label(RichText::new("Nog niet beschikbaar").color(t.text_dim));
                }
            });
        });
}

fn calendar_ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    use calendar::View;
    if ribbon_action(ui, t, "📅", "Nieuwe afspraak", true) {
        let date = app.calendar.focus;
        app.new_appointment(date, 9);
    }
    ui.separator();
    if ribbon_action(ui, t, "◎", "Vandaag", true) {
        app.calendar.focus = calendar::today();
    }
    for (glyph, label, view) in [("▯", "Dag", View::Day), ("▥", "Week", View::Week), ("▦", "Maand", View::Month)] {
        if ribbon_action(ui, t, glyph, label, app.calendar.view != view) {
            app.calendar.view = view;
        }
    }
    ui.separator();
    let has = app.engine.session.selected_appointment.is_some();
    if ribbon_action(ui, t, "🗑", "Verwijderen", has) {
        app.run("appointment.delete", Value::Null);
        app.calendar.editor = None;
    }
    if ribbon_action(ui, t, "⟳", "Vernieuwen", !app.engine.session.item_accounts.is_empty()) {
        app.run("sync.refresh", Value::Null);
    }
}

/// A large ribbon button: glyph above label, drawn by hand as one widget so a screen reader
/// sees a single button named `label`. Returns true when clicked.
pub(crate) fn ribbon_action(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, enabled: bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(72.0, 56.0), if enabled { Sense::click() } else { Sense::hover() });
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let color = if enabled { t.text } else { t.text_dim.gamma_multiply(0.6) };
    let p = ui.painter();
    if r.hovered() && enabled {
        p.rect_filled(rect, CornerRadius::same(4), t.hover.gamma_multiply(0.5));
    }
    p.text(
        egui::pos2(rect.center().x, rect.top() + 18.0),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(22.0),
        if enabled { t.accent } else { color },
    );
    p.text(egui::pos2(rect.center().x, rect.bottom() - 10.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), color);
    enabled && r.clicked()
}

/// A ribbon button that runs a command. `command` `None` draws it disabled (not built yet).
fn ribbon_button(app: &mut MailApp, ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, command: Option<&str>, enabled: bool) {
    let enabled = enabled && command.is_some();
    if let (true, Some(id)) = (ribbon_action(ui, t, glyph, label, enabled), command) {
        app.run(id, Value::Null);
    }
}

fn rail(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("rail").exact_size(56.0).resizable(false).frame(egui::Frame::NONE.fill(t.rail).inner_margin(Margin::symmetric(4, 10))).show(
        ui,
        |ui| {
            ui.vertical_centered(|ui| {
                for m in Module::ALL {
                    let on = app.module == m;
                    let text = RichText::new(m.glyph()).size(20.0).color(if on { t.accent } else { t.text_dim });
                    let b = egui::Button::new(text).fill(if on { t.checked } else { Color32::TRANSPARENT }).min_size(vec2(44.0, 40.0));
                    if ui.add(b).on_hover_text(m.label()).clicked() {
                        app.module = m;
                    }
                    ui.add_space(4.0);
                }
            });
        },
    );
}

fn folder_icon(kind: FolderKind) -> &'static str {
    match kind {
        FolderKind::Inbox => "📥",
        FolderKind::Drafts => "📝",
        FolderKind::Sent => "📤",
        FolderKind::Archive => "🗄",
        FolderKind::Junk => "⚠",
        FolderKind::Deleted => "🗑",
        FolderKind::Other => "📁",
    }
}

fn folder_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("folders")
        .default_size(220.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(Margin::symmetric(8, 10)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            let mut pick = None;
            let mut open_accounts = false;
            for account in &app.engine.session.accounts {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(account.address.email()).strong().color(t.text));
                    let (dot, tip) = match app.engine.session.status.get(&account.id) {
                        Some(AccountStatus::Online) => (t.accent, "Verbonden"),
                        Some(AccountStatus::Connecting) => (t.text_dim, "Verbinden…"),
                        Some(AccountStatus::Error(_)) => (t.flag, "Fout, zie de statusbalk"),
                        _ => (t.border, "Niet verbonden"),
                    };
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, dot);
                    ui.allocate_rect(r, Sense::hover()).on_hover_text(tip);
                });
                ui.add_space(4.0);
                for f in app.engine.session.folders.iter().filter(|f| f.account_id == account.id) {
                    let on = app.engine.session.selected_folder.as_deref() == Some(f.id.as_str());
                    let r = ui.add_sized(
                        [ui.available_width(), 28.0],
                        egui::Button::new("").fill(if on { t.checked } else { Color32::TRANSPARENT }).frame_when_inactive(on),
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &f.name));
                    let rect = r.rect.shrink2(vec2(8.0, 0.0));
                    let p = ui.painter();
                    let name = format!("{}  {}", folder_icon(f.kind), f.name);
                    let font = egui::FontId::proportional(13.5);
                    p.text(rect.left_center(), egui::Align2::LEFT_CENTER, name, font.clone(), t.text);
                    if f.unread > 0 {
                        p.text(rect.right_center(), egui::Align2::RIGHT_CENTER, f.unread.to_string(), font, t.accent_text);
                    }
                    if r.clicked() {
                        pick = Some(f.id.clone());
                    }
                }
            }
            ui.add_space(12.0);
            if ui.add(egui::Button::new(RichText::new("＋ Account toevoegen").color(t.accent_text)).frame(false)).clicked() {
                open_accounts = true;
            }
            if open_accounts {
                app.tab = Tab::File;
            }
            if let Some(id) = pick {
                app.run("folder.select", json!({ "id": id }));
            }
        });
}

fn message_list(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("messages").default_size(360.0).resizable(true).frame(egui::Frame::NONE.fill(t.list).stroke(Stroke::new(1.0, t.border))).show(
        ui,
        |ui| {
            let folder_name = app
                .engine
                .session
                .selected_folder
                .as_deref()
                .and_then(|id| app.engine.session.folder(id))
                .map(|f| f.name.clone())
                .unwrap_or_default();
            egui::Frame::NONE.inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.label(RichText::new(folder_name).size(18.0).strong().color(t.text));
            });
            let mut pick = None;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let list = app.engine.session.visible_messages();
                let loading = app.engine.session.selected_folder.as_deref().is_some_and(|f| app.engine.session.loading.contains(f));
                if list.is_empty() {
                    ui.add_space(24.0);
                    let msg = if loading { "Berichten ophalen…" } else { "Deze map is leeg." };
                    ui.vertical_centered(|ui| ui.label(RichText::new(msg).color(t.text_dim)));
                }
                for m in list {
                    let on = app.engine.session.selected_message.as_deref() == Some(m.id.as_str());
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 68.0), Sense::click());
                    let r = r.on_hover_text(&m.subject);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &m.subject));
                    let p = ui.painter();
                    let fill = if on {
                        t.checked
                    } else if r.hovered() {
                        t.hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    p.rect_filled(rect, CornerRadius::ZERO, fill);
                    if !m.flags.seen {
                        p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height())), CornerRadius::ZERO, t.accent);
                    }
                    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.border));
                    let x = rect.left() + 14.0;
                    let w = rect.width() - 28.0;
                    let from_color = if m.flags.seen { t.text } else { t.accent_text };
                    p.text(
                        egui::pos2(x, rect.top() + 14.0),
                        egui::Align2::LEFT_CENTER,
                        clip(m.from.display(), 34),
                        egui::FontId::proportional(14.0),
                        from_color,
                    );
                    let mut right = dates::short(m.date);
                    if m.has_attachments {
                        right = format!("📎 {right}");
                    }
                    if m.flags.flagged {
                        right = format!("⚑ {right}");
                    }
                    p.text(
                        egui::pos2(x + w, rect.top() + 14.0),
                        egui::Align2::RIGHT_CENTER,
                        right,
                        egui::FontId::proportional(12.0),
                        if m.flags.flagged { t.flag } else { t.text_dim },
                    );
                    p.text(
                        egui::pos2(x, rect.top() + 34.0),
                        egui::Align2::LEFT_CENTER,
                        clip(&m.subject, 48),
                        egui::FontId::proportional(13.0),
                        if m.flags.seen { t.text } else { t.accent_text },
                    );
                    p.text(
                        egui::pos2(x, rect.top() + 52.0),
                        egui::Align2::LEFT_CENTER,
                        clip(&m.preview, 56),
                        egui::FontId::proportional(12.0),
                        t.text_dim,
                    );
                    if r.clicked() {
                        pick = Some(m.id.clone());
                    }
                }
            });
            if let Some(id) = pick {
                app.run("message.select", json!({ "id": id }));
            }
        },
    );
}

fn reading_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(m) = app.engine.session.selected() else {
        ui.centered_and_justified(|ui| ui.label(RichText::new("Selecteer een bericht om het te lezen.").color(t.text_dim).size(15.0)));
        return;
    };
    ui.label(RichText::new(&m.subject).size(20.0).strong().color(t.text));
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 20.0, t.accent);
        let initial = m.from.display().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initial, egui::FontId::proportional(18.0), t.on_accent);
        ui.vertical(|ui| {
            ui.label(RichText::new(format!("{} <{}>", m.from.display(), m.from.email())).color(t.text));
            let to: Vec<&str> = m.to.iter().map(|a| a.display()).collect();
            ui.label(RichText::new(format!("Aan: {}", to.join("; "))).size(12.0).color(t.text_dim));
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.label(RichText::new(dates::long(m.date)).size(12.0).color(t.text_dim));
        });
    });
    ui.add_space(8.0);
    let body = app.engine.session.selected_body().cloned();
    let loading = app.engine.session.loading.contains(&m.id);
    let sample = app.engine.session.is_sample();
    if body.as_ref().is_some_and(|b| b.from_html) {
        // Privacy: HTML is reduced to text; images, scripts and tracking pixels never load.
        egui::Frame::NONE.fill(t.checked).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.label(
                RichText::new("🛡 Dit bericht is als tekst weergegeven. Externe afbeeldingen zijn geblokkeerd om je privacy te beschermen.")
                    .size(12.0)
                    .color(t.text),
            );
        });
        ui.add_space(8.0);
    }
    if let Some(b) = body.as_ref().filter(|b| !b.attachments.is_empty()) {
        ui.horizontal_wrapped(|ui| {
            for a in &b.attachments {
                egui::Frame::NONE
                    .fill(t.panel)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.label(RichText::new(format!("📎 {}  ({})", a.name, size_text(a.size))).size(12.0).color(t.text));
                    });
            }
        });
        ui.add_space(8.0);
    }
    ui.add_space(6.0);
    let text = match (&body, loading, sample) {
        (Some(b), _, _) => b.text.clone(),
        (None, true, _) => "Bericht ophalen…".to_string(),
        (None, false, true) => m.preview.clone(),
        (None, false, false) => String::new(),
    };
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(text).color(if body.is_some() || sample { t.text } else { t.text_dim }).size(14.0)).wrap());
    });
}

/// "12 kB", "3,4 MB".
fn size_text(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} kB", bytes / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)).replace('.', ",")
    }
}

fn status_bar(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::bottom("status")
        .frame(egui::Frame::NONE.fill(t.status_bar).inner_margin(Margin::symmetric(10, 4)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let list = app.engine.session.visible_messages();
                let unread = list.iter().filter(|m| !m.flags.seen).count();
                ui.label(RichText::new(format!("Items: {}   Ongelezen: {}", list.len(), unread)).size(12.0).color(t.text_dim));
                if let Some(n) = &app.notice {
                    let col = if n.ends_with("verzonden.") { t.accent_text } else { t.flag };
                    ui.label(RichText::new(n).size(12.0).color(col));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (txt, col) = if app.engine.session.is_sample() {
                        ("Voorbeeldmail · offline".to_string(), t.text_dim)
                    } else {
                        let statuses = app.engine.session.status.values();
                        let mut worst = ("Verbonden".to_string(), t.text_dim);
                        for s in statuses {
                            match s {
                                AccountStatus::Error(e) => worst = (e.clone(), t.flag),
                                AccountStatus::Connecting if worst.1 != t.flag => worst = ("Verbinden…".to_string(), t.text_dim),
                                _ => {}
                            }
                        }
                        if app.engine.session.has_pending() && worst.1 != t.flag {
                            worst.0 = "Bezig…".to_string();
                        }
                        worst
                    };
                    ui.label(RichText::new(txt).size(12.0).color(col));
                });
            });
        });
}

/// `s` cut to `n` characters with an ellipsis.
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()) }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn harness() -> egui_kittest::Harness<'static, MailApp> {
        egui_kittest::Harness::builder()
            .with_size(vec2(1280.0, 800.0))
            .build_ui_state(|ui, app: &mut MailApp| app.ui(ui), MailApp::new(Engine::sample()))
    }

    #[test]
    fn clicking_a_message_opens_it_and_delete_moves_it() {
        let mut h = harness();
        h.run();
        let subject = "Planning voor donderdag";
        h.get_by_label(subject).click();
        h.run();
        assert_eq!(h.state().engine.session.selected().map(|m| m.subject.as_str()), Some(subject));
        h.get_by_label("Verwijderen").click();
        h.run();
        assert!(h.state().engine.session.selected().is_none());
        assert!(h.state().engine.session.visible_messages().iter().all(|m| m.subject != subject));
    }

    #[test]
    fn tasks_list_completes_saves_and_deletes() {
        let mut h = harness();
        h.state_mut().module = Module::Tasks;
        h.run();
        assert!(h.query_by_label("Offerte drukwerk beantwoorden").is_some());
        assert!(h.query_by_label("Welkomstmail lezen").is_none(), "done tasks are hidden by the Open filter");
        h.get_by_label("Offerte drukwerk beantwoorden voltooid").click();
        h.run();
        assert!(h.state().engine.session.tasks.iter().any(|t| t.title == "Offerte drukwerk beantwoorden" && t.done));
        h.get_by_label("Alles").click();
        h.run();
        h.get_by_label("Welkomstmail lezen").click();
        h.run();
        assert_eq!(h.state().engine.session.selected_task().map(|t| t.title.as_str()), Some("Welkomstmail lezen"));
        h.get_by_label("Nieuwe taak").click();
        h.run();
        h.state_mut().tasks.editor.as_mut().expect("editor").title = "Belasting".into();
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        assert!(h.state().tasks.editor.is_none());
        assert_eq!(h.state().engine.session.selected_task().map(|t| t.title.as_str()), Some("Belasting"));
        h.get_by_label("Verwijderen").click();
        h.run();
        assert!(h.state().engine.session.tasks.iter().all(|t| t.title != "Belasting"));
    }

    #[test]
    fn contacts_list_selects_saves_and_mails() {
        let mut h = harness();
        h.state_mut().module = Module::People;
        h.run();
        h.get_by_label("Sanne de Vries").click();
        h.run();
        assert_eq!(h.state().engine.session.selected_contact().map(|c| c.name.as_str()), Some("Sanne de Vries"));
        h.get_by_label("E-mail sturen").click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| c.to.as_str()), Some("Sanne de Vries <sanne@example.org>"));
        h.state_mut().compose = None;
        h.get_by_label("Nieuw contact").click();
        h.run();
        {
            let e = h.state_mut().contacts.editor.as_mut().expect("editor");
            e.name = "Noor Bakker".into();
            e.emails = "noor@example.org".into();
        }
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        assert!(h.state().contacts.editor.is_none());
        assert!(h.state().engine.session.contacts.iter().any(|c| c.name == "Noor Bakker"));
        h.get_by_label("Verwijderen").click();
        h.run();
        assert!(h.state().engine.session.contacts.iter().all(|c| c.name != "Noor Bakker"));
    }

    #[test]
    fn compose_completes_addresses_from_contacts() {
        let mut h = harness();
        h.run();
        h.get_by_label("Nieuwe e-mail").click();
        h.run();
        h.state_mut().compose.as_mut().expect("compose").to = "sa".into();
        h.run();
        h.get_by_label("Sanne de Vries <sanne@example.org>").click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| c.to.as_str()), Some("Sanne de Vries <sanne@example.org>; "));
    }

    #[test]
    fn calendar_shows_sample_appointments_and_saves_new_ones() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        assert!(h.query_by_label_contains("Overleg ontwerp").is_some(), "today's sample appointment is in the month view");
        h.get_by_label("Week").click();
        h.run();
        assert_eq!(h.state().calendar.view, calendar::View::Week);
        h.get_by_label("Nieuwe afspraak").click();
        h.run();
        h.state_mut().calendar.editor.as_mut().expect("editor").title = "Tandarts".into();
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        let s = &h.state().engine.session;
        assert!(h.state().calendar.editor.is_none());
        assert!(s.appointments.iter().any(|a| a.title == "Tandarts"));
        assert_eq!(s.selected_appointment().map(|a| a.title.as_str()), Some("Tandarts"));
        h.get_by_label("Verwijderen").click();
        h.run();
        assert!(h.state().engine.session.appointments.iter().all(|a| a.title != "Tandarts"));
    }

    #[test]
    fn file_tab_opens_the_backstage_with_the_account_form() {
        let mut h = harness();
        h.run();
        h.get_by_label("Bestand").click();
        h.run();
        assert!(h.query_by_label("Toevoegen en verbinden").is_some());
        h.get_by_label("←  Terug").click();
        h.run();
        assert!(h.query_by_label("Toevoegen en verbinden").is_none());
    }

    #[test]
    fn reply_opens_the_compose_window_prefilled() {
        let mut h = harness();
        h.run();
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        h.get_by_label("Beantwoorden").click();
        h.run();
        let c = h.state().compose.as_ref().expect("compose window");
        assert_eq!(c.subject, "Re: Planning voor donderdag");
        assert!(c.to.contains("sanne@example.org"));
        h.get_by_label("Verzenden").click();
        h.run();
        h.run();
        // The sample account cannot send: the window stays with the reason.
        let c = h.state().compose.as_ref().expect("compose window still open");
        assert!(c.error.as_deref().is_some_and(|e| e.contains("account")));
    }

    #[test]
    fn clip_counts_characters() {
        assert_eq!(clip("Ééénmaal", 4), "Ééé…");
        assert_eq!(clip("kort", 10), "kort");
    }
}
