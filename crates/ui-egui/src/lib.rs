//! Mailcraft's front end: an Outlook-style window drawn with egui in the ArtCraft look.
//!
//! Layout, top to bottom: the tab strip and ribbon, then the module rail (Mail, Calendar,
//! People, Tasks), the folder pane, the message list and the reading pane, then the status bar.
//! Every button runs a command id through [`mailcraft_engine::Session::run`]; nothing here
//! changes the session directly.

mod dates;
mod theme;

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};

use mailcraft_engine::Session;
use mailcraft_model::FolderKind;

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
    pub session: Session,
    pub module: Module,
    tab: Tab,
    /// The last command error, shown in the status bar until the next action.
    notice: Option<String>,
}

impl MailApp {
    pub fn new(session: Session) -> Self {
        MailApp { session, module: Module::Mail, tab: Tab::Home, notice: None }
    }

    /// Runs a command; an error is shown, never raised.
    fn run(&mut self, id: &str, params: Value) {
        self.notice = match self.session.run(id, &params) {
            Ok(()) => None,
            Err(e) => {
                log::warn!("{id}: {e}");
                Some(e.to_string())
            }
        };
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let t = Tokens::for_dark(ui.visuals().dark_mode);
        ribbon(self, ui, &t);
        status_bar(self, ui, &t);
        rail(self, ui, &t);
        if self.module == Module::Mail {
            folder_pane(self, ui, &t);
            message_list(self, ui, &t);
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading).inner_margin(Margin::same(20))).show(ui, |ui| {
                reading_pane(self, ui, &t);
            });
        } else {
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading)).show(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(format!("{} volgt na e-mail.", self.module.label())).color(t.text_dim).size(16.0));
                });
            });
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
                Tab::Home => {
                    let has = app.session.selected_message.is_some();
                    ribbon_button(app, ui, t, "✉", "Nieuwe e-mail", None, false);
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
                    ribbon_button(app, ui, t, "⟳", "Alle mappen verzenden/ontvangen", None, false);
                }
                _ => {
                    ui.label(RichText::new("Nog niet beschikbaar").color(t.text_dim));
                }
            });
        });
}

/// A large ribbon button: glyph above label. `command` `None` draws it disabled (not built yet).
/// Drawn by hand as one widget, so a screen reader sees a single button named `label`.
fn ribbon_button(app: &mut MailApp, ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, command: Option<&str>, enabled: bool) {
    let enabled = enabled && command.is_some();
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
    if let (true, Some(id)) = (enabled && r.clicked(), command) {
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
            for account in &app.session.accounts {
                ui.label(RichText::new(account.address.email()).strong().color(t.text));
                ui.add_space(4.0);
                for f in app.session.folders.iter().filter(|f| f.account_id == account.id) {
                    let on = app.session.selected_folder.as_deref() == Some(f.id.as_str());
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
            if let Some(id) = pick {
                app.run("folder.select", json!({ "id": id }));
            }
        });
}

fn message_list(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("messages").default_size(360.0).resizable(true).frame(egui::Frame::NONE.fill(t.list).stroke(Stroke::new(1.0, t.border))).show(
        ui,
        |ui| {
            let folder_name =
                app.session.selected_folder.as_deref().and_then(|id| app.session.folder(id)).map(|f| f.name.clone()).unwrap_or_default();
            egui::Frame::NONE.inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.label(RichText::new(folder_name).size(18.0).strong().color(t.text));
            });
            let mut pick = None;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let list = app.session.visible_messages();
                if list.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("Deze map is leeg.").color(t.text_dim)));
                }
                for m in list {
                    let on = app.session.selected_message.as_deref() == Some(m.id.as_str());
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
    let Some(m) = app.session.selected() else {
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
    // Privacy: remote content (images, fonts, tracking pixels) is never loaded without consent.
    egui::Frame::NONE.fill(t.checked).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.label(RichText::new("🛡 Externe afbeeldingen zijn geblokkeerd om je privacy te beschermen.").size(12.0).color(t.text));
    });
    ui.add_space(14.0);
    ui.label(RichText::new(&m.preview).color(t.text).size(14.0));
}

fn status_bar(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::bottom("status")
        .frame(egui::Frame::NONE.fill(t.status_bar).inner_margin(Margin::symmetric(10, 4)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let list = app.session.visible_messages();
                let unread = list.iter().filter(|m| !m.flags.seen).count();
                ui.label(RichText::new(format!("Items: {}   Ongelezen: {}", list.len(), unread)).size(12.0).color(t.text_dim));
                if let Some(n) = &app.notice {
                    ui.label(RichText::new(n).size(12.0).color(t.flag));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("Voorbeeldmail · offline").size(12.0).color(t.text_dim));
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
            .build_ui_state(|ui, app: &mut MailApp| app.ui(ui), MailApp::new(Session::sample()))
    }

    #[test]
    fn clicking_a_message_opens_it_and_delete_moves_it() {
        let mut h = harness();
        h.run();
        let subject = "Planning voor donderdag";
        h.get_by_label(subject).click();
        h.run();
        assert_eq!(h.state().session.selected().map(|m| m.subject.as_str()), Some(subject));
        h.get_by_label("Verwijderen").click();
        h.run();
        assert!(h.state().session.selected().is_none());
        assert!(h.state().session.visible_messages().iter().all(|m| m.subject != subject));
    }

    #[test]
    fn other_modules_show_a_placeholder() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        assert!(h.query_by_label_contains("Agenda volgt").is_some());
    }

    #[test]
    fn clip_counts_characters() {
        assert_eq!(clip("Ééénmaal", 4), "Ééé…");
        assert_eq!(clip("kort", 10), "kort");
    }
}
