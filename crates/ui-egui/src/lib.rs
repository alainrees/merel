//! Mailcraft's front end: an Outlook-style window drawn with egui in the ArtCraft look.
//!
//! Layout, top to bottom: the tab strip and ribbon, then the module rail (Mail, Calendar,
//! People, Tasks), the folder pane, the message list and the reading pane, then the status bar.
//! Every button runs a command id through [`mailcraft_engine::Session::run`]; nothing here
//! changes the session directly.

mod attachments;
mod backstage;
pub mod calendar;
pub mod categories;
mod compose;
pub mod contacts;
mod dates;
pub mod folders;
pub mod help;
mod html;
pub mod print;
pub mod quicksteps;
pub mod reminders;
pub mod remote;
pub mod rules;
pub mod tasks;
mod theme;
pub mod view;

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
    Folder,
    SendReceive,
    View,
    Help,
}

impl Tab {
    const ALL: [Tab; 6] = [Tab::File, Tab::Home, Tab::Folder, Tab::SendReceive, Tab::View, Tab::Help];

    fn label(self) -> &'static str {
        match self {
            Tab::File => "Bestand",
            Tab::Home => "Start",
            Tab::Folder => "Map",
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
    /// The message whose body is on screen; when it changes, its pictures leave the cache.
    shown_message: Option<String>,
    /// Pictures on the internet, fetched for the open message only when asked.
    pub remote: remote::RemoteImages,
    /// The account whose drafts the message list shows instead of a folder, if any.
    pub drafts_for: Option<String>,
    /// Reminders the window has already asked attention for, by occurrence.
    pub announced_reminders: std::collections::HashSet<(String, i64)>,
    /// The rules window, while it is open.
    pub rules: Option<rules::State>,
    /// The quick steps window, when it is open.
    pub quick_steps: Option<quicksteps::State>,
    /// Whether the "Verplaatsen naar" window for the selected message is open.
    pub move_to: bool,
    /// The window for a new, renamed or removed folder, while it is open.
    pub folder_edit: Option<folders::FolderEdit>,
    /// The "Alle categorieën" window, while it is open.
    pub categories: Option<categories::Manager>,
    /// The open help window ("Help" tab), if any.
    pub help: Option<help::HelpWindow>,
    /// What the "Beeld" tab chose; read from `view.json` at start.
    pub view: view::ViewPrefs,
    /// Whether the saved theme has been handed to egui yet (on the first frame).
    theme_applied: bool,
}

/// A message being dragged from the list, to drop on a folder.
struct DraggedMessage(String);

impl MailApp {
    pub fn new(mut engine: Engine) -> Self {
        let view = view::ViewPrefs::load(engine.config_path());
        engine.session.sort = view.sort;
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
            shown_message: None,
            remote: remote::RemoteImages::default(),
            drafts_for: None,
            announced_reminders: std::collections::HashSet::new(),
            rules: None,
            quick_steps: None,
            move_to: false,
            folder_edit: None,
            categories: None,
            help: None,
            view,
            theme_applied: false,
        }
    }

    /// Runs a command; an error is shown, never raised. Deleting a message that is in the
    /// Deleted folder already would remove it for good, so that asks first.
    fn run(&mut self, id: &str, params: Value) {
        if id == "message.delete" && self.selected_is_deleted() {
            self.delete_forever();
            return;
        }
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
        if !self.theme_applied {
            ui.ctx().set_theme(self.view.theme.preference());
            self.theme_applied = true;
        }
        let t = Tokens::for_dark(ui.visuals().dark_mode);
        // Decoders for pictures embedded in mail; a no-op after the first frame.
        egui_extras::install_image_loaders(ui.ctx());
        self.engine.poll();
        let arrivals = self.engine.session.take_arrivals();
        if arrivals > 0 {
            self.notice =
                Some(if arrivals == 1 { "1 nieuw bericht ontvangen.".to_string() } else { format!("{arrivals} nieuwe berichten ontvangen.") });
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
        let hits = self.engine.session.take_rule_hits();
        if hits > 0 {
            self.notice = Some(rules::rule_notice(hits));
        }
        // Wake up now and then so the engine's refresh timer runs while nothing happens; faster
        // while something is on its way.
        let wake = if self.engine.session.has_pending() { std::time::Duration::from_millis(250) } else { std::time::Duration::from_secs(5) };
        ui.ctx().request_repaint_after(wake);
        ribbon(self, ui, &t);
        status_bar(self, ui, &t);
        let ctx = ui.ctx().clone();
        compose::show(self, &ctx, &t);
        calendar::window(self, &ctx, &t);
        reminders::window(self, &ctx, &t);
        rules::window(self, &ctx, &t);
        quicksteps::window(self, &ctx, &t);
        move_window(self, &ctx, &t);
        folders::window(self, &ctx, &t);
        categories::window(self, &ctx, &t);
        help::window(self, &ctx, &t);
        drag_preview(self, &ctx, &t);
        if self.tab == Tab::Home && self.module == Module::Mail {
            shortcuts(self, &ctx);
        }
        if self.tab == Tab::File {
            backstage::show(self, ui, &t);
            return;
        }
        rail(self, ui, &t);
        if self.module == Module::Mail {
            folder_pane(self, ui, &t);
            let frame = egui::Frame::NONE.fill(t.list).stroke(Stroke::new(1.0, t.border));
            match self.view.reading_pane {
                view::ReadingPane::Right => {
                    egui::Panel::left("messages").default_size(360.0).resizable(true).frame(frame).show(ui, |ui| message_list(self, ui, &t));
                }
                view::ReadingPane::Bottom => {
                    egui::Panel::top("messages").default_size(320.0).resizable(true).frame(frame).show(ui, |ui| message_list(self, ui, &t));
                }
            }
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

/// The search box's widget id, so a shortcut can put the cursor there.
const SEARCH_BOX: &str = "message_search_box";

/// Keyboard shortcuts for the mail module, the way Outlook has them. They work while no text
/// field has the keyboard and no message is being written, so typing never triggers one;
/// Ctrl+E (Cmd+E on a Mac) is the exception, as it moves the cursor to the search box.
fn shortcuts(app: &mut MailApp, ctx: &egui::Context) {
    use egui::{Key, Modifiers};
    if app.compose.is_some() {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::E)) {
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(SEARCH_BOX)));
        return;
    }
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let has = app.engine.session.selected_message.is_some();
    let mut compose = None;
    let mut command = None;
    let mut step = 0;
    let mut quick: Option<usize> = None;
    const DIGITS: [Key; 9] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    // On a Mac the big delete key is Backspace, so there Backspace deletes and Ctrl+E archives,
    // as in Outlook for Mac; elsewhere Backspace archives and Delete deletes, as in Outlook.
    const MAC: bool = cfg!(target_os = "macos");
    ctx.input_mut(|i| {
        if let Some(n) = DIGITS.iter().position(|k| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, *k)) {
            quick = Some(n);
        } else if i.consume_key(Modifiers::COMMAND, Key::N) {
            compose = Some(ComposeKind::New);
        } else if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::R) {
            compose = Some(ComposeKind::ReplyAll);
        } else if i.consume_key(Modifiers::COMMAND, Key::R) {
            compose = Some(ComposeKind::Reply);
        } else if i.consume_key(Modifiers::COMMAND, Key::F) {
            compose = Some(ComposeKind::Forward);
        } else if i.consume_key(Modifiers::COMMAND, Key::P) {
            command = Some("message.print");
        } else if i.consume_key(Modifiers::NONE, Key::Delete) || (MAC && i.consume_key(Modifiers::NONE, Key::Backspace)) {
            command = Some("message.delete");
        } else if (!MAC && i.consume_key(Modifiers::NONE, Key::Backspace)) || (MAC && i.consume_key(Modifiers::CTRL, Key::E)) {
            command = Some("message.archive");
        } else if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::V) {
            command = Some("message.move_window");
        } else if i.consume_key(Modifiers::COMMAND, Key::Q) {
            command = Some("message.mark_read");
        } else if i.consume_key(Modifiers::COMMAND, Key::U) {
            command = Some("message.mark_unread");
        } else if i.consume_key(Modifiers::NONE, Key::F5) {
            command = Some("folder.refresh");
        } else if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            step = 1;
        } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            step = -1;
        } else if i.consume_key(Modifiers::NONE, Key::Escape) && app.engine.session.is_searching() {
            command = Some("search.clear");
        }
    });
    if let Some(kind) = compose {
        if kind == ComposeKind::New || has {
            app.open_compose(kind);
        }
        return;
    }
    if let Some(n) = quick {
        app.run_quick_step(n);
        return;
    }
    match command {
        Some("search.clear") => app.engine.session.search.clear(),
        Some("folder.refresh") => app.run("folder.refresh", Value::Null),
        Some("message.move_window") if has => app.move_to = true,
        Some("message.print") if app.can_print() => app.print_selected(),
        Some(id) if has => app.run(id, Value::Null),
        _ => {}
    }
    if step != 0 {
        let ids: Vec<String> = app.engine.session.visible_messages().iter().map(|m| m.id.clone()).collect();
        let current = app.engine.session.selected_message.as_deref().and_then(|s| ids.iter().position(|id| id == s));
        let next = match (current, step) {
            (None, _) => ids.first(),
            (Some(i), 1) => ids.get(i + 1),
            (Some(i), _) => i.checked_sub(1).and_then(|j| ids.get(j)),
        };
        if let Some(id) = next.cloned() {
            app.run("message.select", serde_json::json!({ "id": id }));
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
                    if ribbon_action_hint(ui, t, "✉", "Nieuwe e-mail", true, Some(&key_hint("N"))) {
                        open = Some(ComposeKind::New);
                    }
                    ui.separator();
                    if ribbon_action_hint(ui, t, "↩", "Beantwoorden", has, Some(&key_hint("R"))) {
                        open = Some(ComposeKind::Reply);
                    }
                    if ribbon_action_hint(ui, t, "↞", "Allen beantwoorden", has, Some(&key_hint("Shift+R"))) {
                        open = Some(ComposeKind::ReplyAll);
                    }
                    if ribbon_action_hint(ui, t, "↪", "Doorsturen", has, Some(&key_hint("F"))) {
                        open = Some(ComposeKind::Forward);
                    }
                    if let Some(kind) = open {
                        app.open_compose(kind);
                    }
                    ui.separator();
                    ribbon_button(app, ui, t, "🗑", "Verwijderen", ("message.delete", Some("Delete")), has);
                    ribbon_button(
                        app,
                        ui,
                        t,
                        "🗄",
                        "Archiveren",
                        ("message.archive", Some(if cfg!(target_os = "macos") { "Ctrl+E" } else { "Backspace" })),
                        has,
                    );
                    ribbon_button(app, ui, t, "⚠", "Ongewenst", ("message.junk", None), has);
                    if ribbon_action_hint(ui, t, "📂", "Verplaatsen", has, Some(&key_hint("Shift+V"))) {
                        app.move_to = true;
                    }
                    ui.separator();
                    ribbon_button(app, ui, t, "◉", "Gelezen", ("message.mark_read", Some(&key_hint("Q"))), has);
                    ribbon_button(app, ui, t, "○", "Ongelezen", ("message.mark_unread", Some(&key_hint("U"))), has);
                    ribbon_button(app, ui, t, "⚑", "Vlag", ("message.flag", None), has);
                    categories::menu(app, ui, t);
                    ui.separator();
                    quicksteps::ribbon(app, ui, t);
                    ui.separator();
                    if ribbon_action_hint(ui, t, "⚙", "Regels", true, Some("Mail automatisch verplaatsen of markeren")) {
                        app.open_rules(false);
                    }
                }
                Tab::Folder if app.module == Module::Mail => folders::ribbon(app, ui, t),
                Tab::View => view::ribbon(app, ui, t),
                Tab::Help => help::ribbon(app, ui, t),
                Tab::SendReceive => {
                    let can = !app.engine.session.is_sample() && app.engine.session.selected_folder.is_some();
                    ribbon_button(app, ui, t, "⟳", "Vernieuwen", ("folder.refresh", Some("F5")), can);
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

/// "Cmd+N" on a Mac, "Ctrl+N" elsewhere.
pub(crate) fn key_hint(key: &str) -> String {
    let modifier = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
    format!("{modifier}+{key}")
}

/// A large ribbon button: glyph above label, drawn by hand as one widget so a screen reader
/// sees a single button named `label`. Returns true when clicked.
pub(crate) fn ribbon_action(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, enabled: bool) -> bool {
    ribbon_action_hint(ui, t, glyph, label, enabled, None)
}

/// [`ribbon_action`] with the keyboard shortcut shown on hover.
fn ribbon_action_hint(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, enabled: bool, shortcut: Option<&str>) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(72.0, 56.0), if enabled { Sense::click() } else { Sense::hover() });
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let r = match shortcut {
        Some(s) => r.on_hover_text(format!("{label} ({s})")),
        None => r,
    };
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

/// A ribbon button that runs a command; `action` is the command id with the shortcut shown
/// on hover, if it has one.
fn ribbon_button(app: &mut MailApp, ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, action: (&str, Option<&str>), enabled: bool) {
    let (command, shortcut) = action;
    if ribbon_action_hint(ui, t, glyph, label, enabled, shortcut) {
        app.run(command, Value::Null);
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
            let mut pick_drafts = None;
            let mut drop_on: Option<(String, String)> = None;
            let mut menu_on: Option<(egui::Response, String, FolderKind)> = None;
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
                    // A message dragged from the list lands here.
                    if r.dnd_hover_payload::<DraggedMessage>().is_some() {
                        p.rect_stroke(r.rect, CornerRadius::same(4), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                    }
                    if let Some(dragged) = r.dnd_release_payload::<DraggedMessage>() {
                        drop_on = Some((dragged.0.clone(), f.id.clone()));
                    }
                    if r.clicked() {
                        pick = Some(f.id.clone());
                    }
                    if r.secondary_clicked() || r.context_menu_opened() {
                        menu_on = Some((r, f.id.clone(), f.kind));
                    }
                }
                // Messages put aside on this computer, next to the server's folders.
                let drafts = app.engine.session.drafts_of(&account.id).len();
                if drafts > 0 {
                    let on = app.drafts_for.as_deref() == Some(account.id.as_str());
                    let r = ui.add_sized(
                        [ui.available_width(), 28.0],
                        egui::Button::new("").fill(if on { t.checked } else { Color32::TRANSPARENT }).frame_when_inactive(on),
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Concepten op deze computer"));
                    let r = r.on_hover_text("Berichten die je opzij hebt gezet; ze staan alleen op deze computer.");
                    let rect = r.rect.shrink2(vec2(8.0, 0.0));
                    let p = ui.painter();
                    let font = egui::FontId::proportional(13.5);
                    p.text(rect.left_center(), egui::Align2::LEFT_CENTER, "📝  Concepten (deze computer)", font.clone(), t.text);
                    p.text(rect.right_center(), egui::Align2::RIGHT_CENTER, drafts.to_string(), font, t.text_dim);
                    if r.clicked() {
                        pick_drafts = Some(account.id.clone());
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
                app.drafts_for = None;
                app.run("folder.select", json!({ "id": id }));
            }
            if let Some(id) = pick_drafts {
                app.drafts_for = Some(id);
            }
            if let Some((message, folder)) = drop_on {
                app.run("message.move", json!({ "id": message, "folder_id": folder }));
            }
            if let Some((r, id, kind)) = menu_on {
                folders::context_menu(app, &r, &id, kind);
            }
        });
}

/// The "Verplaatsen naar" window: the folders of the selected message's account.
fn move_window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    if !app.move_to {
        return;
    }
    let Some(m) = app.engine.session.selected().cloned() else {
        app.move_to = false;
        return;
    };
    let account = app.engine.session.folder(&m.folder_id).map(|f| f.account_id.clone()).unwrap_or_default();
    let folders: Vec<(String, String, FolderKind)> = app
        .engine
        .session
        .folders
        .iter()
        .filter(|f| f.account_id == account && f.id != m.folder_id)
        .map(|f| (f.id.clone(), f.name.clone(), f.kind))
        .collect();
    let mut target = None;
    let mut close = false;
    egui::Window::new("Verplaatsen naar").collapsible(false).resizable(false).default_width(280.0).show(ctx, |ui| {
        ui.label(RichText::new(clip(&m.subject, 48)).size(12.0).color(t.text_dim));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (id, name, kind) in &folders {
                if ui.add_sized([260.0, 26.0], egui::Button::new(format!("{} {name}", folder_icon(*kind)))).clicked() {
                    target = Some(id.clone());
                }
            }
        });
        ui.add_space(6.0);
        if ui.button("Annuleren").clicked() {
            close = true;
        }
    });
    if let Some(folder) = target {
        app.run("message.move", json!({ "folder_id": folder }));
        close = true;
    }
    if close {
        app.move_to = false;
    }
}

/// The subject that follows the pointer while a message is dragged to a folder.
fn drag_preview(app: &MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(dragged) = egui::DragAndDrop::payload::<DraggedMessage>(ctx) else { return };
    let Some(pos) = ctx.pointer_interact_pos() else { return };
    let subject = app.engine.session.message(&dragged.0).map(|m| clip(&m.subject, 40)).unwrap_or_default();
    egui::Area::new(egui::Id::new("drag-preview")).fixed_pos(pos + vec2(12.0, 12.0)).order(egui::Order::Tooltip).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.list).show(ui, |ui| {
            ui.label(RichText::new(format!("✉ {subject}")).color(t.text));
        });
    });
}

/// The drafts of one account in place of the message list: open one to go on writing, or drop it.
fn drafts_list(app: &mut MailApp, ui: &mut Ui, t: &Tokens, account_id: &str) {
    let drafts: Vec<mailcraft_model::StoredDraft> = app.engine.session.drafts_of(account_id).into_iter().cloned().collect();
    egui::Frame::NONE.inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.label(RichText::new(format!("Concepten ({})", drafts.len())).size(18.0).strong().color(t.text));
        ui.label(
            RichText::new("Berichten die je opzij hebt gezet. Ze staan alleen op deze computer, in de versleutelde offline kopie.")
                .size(12.0)
                .color(t.text_dim),
        );
    });
    let mut open = None;
    let mut delete = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        if drafts.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Geen concepten.").color(t.text_dim)));
        }
        for d in &drafts {
            egui::Frame::NONE.inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let subject = if d.subject.is_empty() { "(Geen onderwerp)" } else { d.subject.as_str() };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(clip(subject, 48)).strong().color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(dates::short(d.updated)).size(12.0).color(t.text_dim));
                    });
                });
                let to = if d.to.trim().is_empty() { "(nog geen ontvanger)" } else { d.to.as_str() };
                ui.label(RichText::new(format!("Aan: {}", clip(to, 56))).size(12.0).color(t.text_dim));
                let preview: String = d.body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
                if !preview.is_empty() {
                    ui.label(RichText::new(clip(&preview, 72)).size(12.0).color(t.text_dim));
                }
                ui.horizontal(|ui| {
                    if ui.button("Openen").clicked() {
                        open = Some(d.id.clone());
                    }
                    if ui.button("Concept verwijderen").clicked() {
                        delete = Some(d.id.clone());
                    }
                });
            });
            ui.separator();
        }
    });
    if let Some(id) = open {
        app.open_draft(&id);
    }
    if let Some(id) = delete {
        app.run("draft.delete", json!({ "id": id }));
    }
}

/// The message list: title, search box and rows; the caller puts it in a panel.
fn message_list(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    {
        {
            if let Some(account_id) = app.drafts_for.clone() {
                drafts_list(app, ui, t, &account_id);
                return;
            }
            let folder_name = app
                .engine
                .session
                .selected_folder
                .as_deref()
                .and_then(|id| app.engine.session.folder(id))
                .map(|f| f.name.clone())
                .unwrap_or_default();
            let searching = app.engine.session.is_searching();
            let count = if searching { Some(app.engine.session.visible_messages().len()) } else { None };
            let mut server_search = false;
            egui::Frame::NONE.inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                let title = match count {
                    Some(n) => format!("Zoekresultaten ({n})"),
                    None => folder_name,
                };
                ui.label(RichText::new(title).size(18.0).strong().color(t.text));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let session = &mut app.engine.session;
                    let hint = if session.search_all { "Zoeken in alle mappen" } else { "Zoeken in deze map" };
                    let width = (ui.available_width() - 150.0).max(80.0);
                    ui.add(egui::TextEdit::singleline(&mut session.search).id(egui::Id::new(SEARCH_BOX)).hint_text(hint).desired_width(width));
                    if searching && ui.small_button("✕").on_hover_text("Zoekopdracht wissen").clicked() {
                        session.search.clear();
                    }
                    ui.checkbox(&mut session.search_all, "Alle mappen")
                        .on_hover_text("Zoek in alle mappen van alle accounts, in de berichten die Mailcraft al heeft opgehaald.");
                });
                // The server can look where Mailcraft has not: older mail, and the text itself.
                let session = &app.engine.session;
                if session.can_search_server() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let busy = session.searching_server();
                        match session.current_hits() {
                            Some(hits) => {
                                let n = hits.ids.len();
                                let found = if n == 1 { "1 bericht".to_string() } else { format!("{n} berichten") };
                                ui.label(RichText::new(format!("De server vond {found} in deze map.")).size(12.0).color(t.text_dim));
                            }
                            None => {
                                let label = if busy { "Zoeken op de server…" } else { "Zoek op de server" };
                                if ui
                                    .add_enabled(!busy, egui::Button::new(label))
                                    .on_hover_text("Laat de mailserver in deze map zoeken: ook in berichten die Mailcraft nog niet heeft opgehaald en in de tekst van de berichten.")
                                    .clicked()
                                {
                                    server_search = true;
                                }
                            }
                        }
                    });
                    if let Some(e) = &session.search_error {
                        ui.label(RichText::new(format!("Zoeken op de server is mislukt. {e}")).size(12.0).color(t.flag));
                    }
                }
            });
            if server_search {
                let _ = app.engine.run("search.server", &serde_json::Value::Null);
            }
            let mut pick = None;
            let mut older = false;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let list = app.engine.session.visible_messages();
                let loading = app.engine.session.selected_folder.as_deref().is_some_and(|f| app.engine.session.loading.contains(f));
                if list.is_empty() {
                    ui.add_space(24.0);
                    let msg = if searching && app.engine.session.current_hits().is_some() {
                        "Geen berichten gevonden, ook niet op de server."
                    } else if searching {
                        "Geen berichten gevonden. Mailcraft zoekt in de berichten die het al heeft opgehaald."
                    } else if loading {
                        "Berichten ophalen…"
                    } else {
                        "Deze map is leeg."
                    };
                    ui.vertical_centered(|ui| ui.label(RichText::new(msg).color(t.text_dim)));
                }
                let show_folder = searching && app.engine.session.search_all;
                let compact = app.view.compact;
                let row_height = if compact { 48.0 } else { 68.0 };
                for m in list {
                    let on = app.engine.session.selected_message.as_deref() == Some(m.id.as_str());
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click_and_drag());
                    r.dnd_set_drag_payload(DraggedMessage(m.id.clone()));
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
                    let squares = categories::squares(app, p, egui::pos2(x + w, rect.top() + 34.0), &m.categories);
                    p.text(
                        egui::pos2(x, rect.top() + 34.0),
                        egui::Align2::LEFT_CENTER,
                        clip(&m.subject, if squares > 0.0 { 40 } else { 48 }),
                        egui::FontId::proportional(13.0),
                        if m.flags.seen { t.text } else { t.accent_text },
                    );
                    if !compact {
                        let preview = match (show_folder, app.engine.session.folder(&m.folder_id)) {
                            (true, Some(f)) => format!("{} · {}", f.name, m.preview),
                            _ => m.preview.clone(),
                        };
                        p.text(
                            egui::pos2(x, rect.top() + 52.0),
                            egui::Align2::LEFT_CENTER,
                            clip(&preview, 56),
                            egui::FontId::proportional(12.0),
                            t.text_dim,
                        );
                    }
                    if r.clicked() {
                        pick = Some(m.id.clone());
                    }
                }
                let selected = app.engine.session.selected_folder.clone();
                if !searching && let Some(folder) = selected {
                    if loading && !app.engine.session.visible_messages().is_empty() {
                        ui.add_space(10.0);
                        ui.vertical_centered(|ui| ui.label(RichText::new("Berichten ophalen…").size(12.0).color(t.text_dim)));
                        ui.add_space(10.0);
                    } else if app.engine.session.can_load_older(&folder) {
                        ui.add_space(10.0);
                        ui.vertical_centered(|ui| {
                            if ui.button("Oudere berichten laden").on_hover_text("Haalt de volgende 200 berichten van de server.").clicked() {
                                older = true;
                            }
                        });
                        ui.add_space(10.0);
                    }
                }
            });
            if let Some(id) = pick {
                app.run("message.select", json!({ "id": id }));
            }
            if older {
                app.run("folder.older", Value::Null);
            }
        }
    }
}

fn reading_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(m) = app.engine.session.selected().cloned() else {
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
    let message_id = m.id.clone();
    let names = m.categories.clone();
    if !names.is_empty() {
        categories::chips(app, ui, t, &message_id, &names);
        ui.add_space(6.0);
    }
    let body = app.engine.session.selected_body().cloned();
    let loading = app.engine.session.loading.contains(&m.id);
    let sample = app.engine.session.is_sample();
    if app.shown_message.as_deref() != Some(message_id.as_str()) {
        // The pictures of the previous message are not needed any more.
        ui.ctx().forget_all_images();
        app.remote.reset();
        app.shown_message = Some(message_id.clone());
    }
    let rich = body.as_ref().and_then(|b| b.rich.as_ref());
    let blocked = rich.map(|d| d.remote_images).unwrap_or(0);
    let allowed = app.remote.allowed(&message_id);
    // Privacy: nothing in a message runs or loads; pictures on the internet stay blocked until
    // the person asks for them, for this one message.
    let notice = match (rich, body.as_ref().is_some_and(|b| b.from_html)) {
        (Some(_), _) if blocked > 0 && allowed => Some("🛡 De afbeeldingen van internet in dit bericht worden geladen omdat je daarom vroeg."),
        (Some(_), _) if blocked > 0 => Some("🛡 Afbeeldingen van internet zijn geblokkeerd om je privacy te beschermen."),
        (Some(_), _) => None,
        (None, true) => Some("🛡 Dit bericht is als tekst weergegeven. Externe afbeeldingen zijn geblokkeerd om je privacy te beschermen."),
        (None, false) => None,
    };
    if let Some(notice) = notice {
        let mut load = false;
        egui::Frame::NONE.fill(t.checked).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(notice).size(12.0).color(t.text));
                if blocked > 0 && !allowed {
                    load = ui
                        .small_button("Afbeeldingen laden")
                        .on_hover_text("Haalt de afbeeldingen van dit ene bericht op van internet. De afzender kan daaraan zien dat je het bericht hebt geopend.")
                        .clicked();
                }
            });
        });
        if load && let Some(doc) = rich {
            app.remote.allow(ui.ctx(), &message_id, doc);
        }
        ui.add_space(8.0);
    }
    if let Some(b) = body.as_ref().filter(|b| !b.attachments.is_empty()) {
        let mut save = None;
        ui.horizontal_wrapped(|ui| {
            for a in &b.attachments {
                let chip = egui::Button::new(RichText::new(format!("📎 {}  ({})", a.name, size_text(a.size))).size(12.0).color(t.text))
                    .fill(t.panel)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(4));
                if ui.add(chip).on_hover_text("Opslaan in je map Downloads. Mailcraft opent bijlagen niet zelf.").clicked() {
                    save = Some(a.clone());
                }
            }
        });
        if let Some(a) = save {
            app.notice = Some(match attachments::downloads_dir() {
                Some(dir) => match attachments::save_to(&dir, &a.name, &a.bytes) {
                    Ok(path) => format!("Bijlage opgeslagen als {}", path.display()),
                    Err(e) => format!("Bijlage niet opgeslagen: {e}"),
                },
                None => "Bijlage niet opgeslagen: geen map Downloads gevonden.".into(),
            });
        }
        ui.add_space(8.0);
    }
    ui.add_space(6.0);
    if let Some(b) = body.as_ref()
        && let Some(doc) = b.rich.as_ref()
    {
        let remote = app.remote.snapshot(&message_id);
        let clicked =
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| html::show(ui, t, doc, &b.images, &message_id, remote.as_ref())).inner;
        if let Some(html::Clicked::Mailto(address)) = clicked {
            let signature = app.engine.session.current_account().map(|a| a.signature.clone()).unwrap_or_default();
            let mut c = Compose::new(ComposeKind::New, None).with_signature(&signature);
            c.to = address;
            app.compose = Some(c);
        }
        return;
    }
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
pub(crate) fn size_text(bytes: usize) -> String {
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
                    let col = if n.ends_with("verzonden.")
                        || n.ends_with("ontvangen.")
                        || n.starts_with("Bijlage opgeslagen")
                        || n.starts_with("Concept bewaard")
                        || n.starts_with("Regels toegepast")
                    {
                        t.accent_text
                    } else {
                        t.flag
                    };
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
                        let queued: usize = app.engine.session.queued.values().sum();
                        match queued {
                            0 => {}
                            1 => worst = ("Geen verbinding; 1 wijziging wacht tot de verbinding terug is".to_string(), t.flag),
                            n => worst = (format!("Geen verbinding; {n} wijzigingen wachten tot de verbinding terug is"), t.flag),
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
    fn the_server_can_be_asked_to_search_a_connected_folder() {
        use mailcraft_engine::{Event, RemoteFolder, RemoteHeader};
        let mut engine = Engine::sample();
        let account = {
            let mut f = crate::backstage::AccountForm::default();
            f.email = "alain@voorbeeld.nl".into();
            f.password = "x".into();
            f.suggest();
            f.account().expect("account")
        };
        let id = account.id.clone();
        engine.add_account(account, mailcraft_secrets::Secret::new("x")).expect("add");
        engine.session.apply(Event::Connected {
            account_id: id.clone(),
            folders: vec![RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: mailcraft_model::FolderKind::Inbox, selectable: true }],
            syncs_items: false,
        });
        // Connecting queued a fetch of the inbox; pretend it came back empty.
        engine.session.take_jobs();
        engine.session.loading.clear();
        let folder = format!("{id}/INBOX");
        engine.session.selected_folder = Some(folder.clone());
        engine.session.search = "factuur".into();
        let mut h =
            egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), MailApp::new(engine));
        h.run();
        h.get_by_label("Zoek op de server").click();
        // The list repaints on a timer while a fetch is in flight, so a bounded number of steps.
        h.run_steps(2);
        assert!(h.state().engine.session.searching_server());
        h.get_by_label("Zoeken op de server…");
        let hit = RemoteHeader {
            id: "7".into(),
            seen: false,
            flagged: false,
            answered: false,
            size: 1,
            has_attachments: false,
            categories: Vec::new(),
            header: mailcraft_mime::parse_header(b"From: boekhouding@voorbeeld.nl\r\nSubject: Jaaroverzicht\r\n\r\n"),
            date: 1_700_000_000,
        };
        h.state_mut().engine.session.apply(Event::Found { folder_id: folder, query: "factuur".into(), result: Ok(vec![hit]) });
        h.run_steps(2);
        h.get_by_label("Jaaroverzicht");
        h.get_by_label("De server vond 1 bericht in deze map.");
        assert!(h.query_by_label("Zoek op de server").is_none());
    }

    #[test]
    fn a_closed_message_is_kept_as_a_draft_and_can_be_reopened() {
        let mut h = harness();
        h.run();
        h.state_mut().open_compose(ComposeKind::New);
        h.run();
        h.get_by_label("Opslaan als concept").click();
        h.run();
        assert!(h.state().compose.is_none() && h.state().engine.session.drafts.is_empty(), "an untouched window leaves nothing");
        h.state_mut().open_compose(ComposeKind::New);
        h.run();
        if let Some(c) = h.state_mut().compose.as_mut() {
            c.subject = "Plan voor morgen".into();
            c.to = "sanne@".into();
        }
        h.get_by_label("Opslaan als concept").click();
        h.run();
        assert!(h.state().compose.is_none());
        assert_eq!(h.state().engine.session.drafts.len(), 1);
        assert!(h.state().notice.as_deref().is_some_and(|n| n.starts_with("Concept bewaard")));
        h.get_by_label("Concepten op deze computer").click();
        h.run();
        assert!(h.state().drafts_for.is_some());
        h.get_by_label("Plan voor morgen");
        h.get_by_label("Openen").click();
        h.run();
        let c = h.state().compose.as_ref().expect("the draft opens");
        assert_eq!((c.subject.as_str(), c.to.as_str()), ("Plan voor morgen", "sanne@"));
        assert!(c.stored.is_some());
        h.state_mut().compose = None;
        h.get_by_label("Concept verwijderen").click();
        h.run();
        assert!(h.state().engine.session.drafts.is_empty());
        h.get_by_label("Geen concepten.");
    }

    #[test]
    fn keyboard_shortcuts_walk_the_list_reply_delete_and_find() {
        use egui::{Key, Modifiers};
        let mut h = harness();
        h.run();
        let ids: Vec<String> = h.state().engine.session.visible_messages().iter().map(|m| m.id.clone()).collect();
        assert!(ids.len() >= 2);
        h.state_mut().engine.session.selected_message = None;
        h.key_press(Key::ArrowDown);
        h.run();
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some(ids[0].as_str()), "Down selects the first message");
        h.key_press(Key::ArrowDown);
        h.run();
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some(ids[1].as_str()));
        h.key_press(Key::ArrowUp);
        h.run();
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some(ids[0].as_str()));

        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::R);
        h.run();
        let c = h.state().compose.as_ref().expect("reply all opens");
        assert!(c.subject.starts_with("Re:") && !c.to.is_empty());
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some(ids[0].as_str()), "keys do nothing while a message is being written");
        h.state_mut().compose = None;
        h.key_press_modifiers(Modifiers::COMMAND, Key::N);
        h.run();
        assert!(h.state().compose.as_ref().is_some_and(|c| c.subject.is_empty()), "Ctrl+N starts a new message");
        h.state_mut().compose = None;

        h.key_press(Key::Delete);
        h.run();
        let moved = h.state().engine.session.message(&ids[0]).map(|m| m.folder_id.clone()).unwrap_or_default();
        assert!(moved.to_lowercase().contains("deleted") || moved.contains("Trash"), "{moved}");

        #[cfg(not(target_os = "macos"))]
        {
            h.state_mut().engine.session.selected_message = Some(ids[1].clone());
            h.key_press(Key::Backspace);
            h.run();
            let moved = h.state().engine.session.message(&ids[1]).map(|m| m.folder_id.clone()).unwrap_or_default();
            assert!(moved.to_lowercase().contains("archive"), "Backspace archives, as in Outlook: {moved}");
        }

        h.key_press_modifiers(Modifiers::COMMAND, Key::E);
        h.run();
        assert_eq!(h.ctx.memory(|m| m.focused()), Some(egui::Id::new(SEARCH_BOX)), "Ctrl+E puts the cursor in the search box");
        h.ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(SEARCH_BOX)));
        h.state_mut().engine.session.search = "planning".into();
        h.run();
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().engine.session.search.is_empty(), "Escape clears the search");
    }

    #[test]
    fn the_search_box_narrows_the_list_and_can_be_cleared() {
        let mut h = harness();
        h.run();
        let all = h.state().engine.session.visible_messages().len();
        h.state_mut().engine.session.search = "donderdag".into();
        h.run();
        assert!(h.query_by_label("Zoekresultaten (1)").is_some());
        assert!(h.query_by_label("Planning voor donderdag").is_some());
        assert_eq!(h.state().engine.session.visible_messages().len(), 1);
        h.get_by_label("✕").click();
        h.run();
        assert!(!h.state().engine.session.is_searching());
        assert_eq!(h.state().engine.session.visible_messages().len(), all);
        h.state_mut().engine.session.search = "niets-hiervan".into();
        h.run();
        assert!(h.query_by_label_contains("Geen berichten gevonden").is_some());
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
    fn a_weekly_appointment_shows_every_week_and_one_time_can_be_skipped() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.view = calendar::View::Week;
        h.run();
        let weekly = h.state().engine.session.appointments.iter().find(|a| a.title == "Weekstart").expect("sample series").clone();
        let series_start = weekly.start;
        assert_eq!(h.query_all_by_label_contains("Weekstart").count(), 1, "one occurrence a week");
        // Next week shows the series again, with its own start.
        let focus = h.state().calendar.focus;
        h.state_mut().calendar.focus = focus.checked_add_days(chrono::Days::new(7)).expect("next week");
        h.run();
        assert_eq!(h.query_all_by_label_contains("Weekstart").count(), 1);
        h.get_by_label_contains("Weekstart").click();
        h.run();
        let editor = h.state().calendar.editor.as_ref().expect("the series opens");
        assert_eq!(editor.repeat, Some(mailcraft_model::Frequency::Weekly));
        let clicked = editor.occurrence.expect("the clicked occurrence is remembered");
        assert!(clicked > series_start && clicked - series_start <= 8 * 86_400, "it is next week's, not the first");
        h.get_by_label("Deze keer overslaan").click();
        h.run();
        assert!(h.state().calendar.editor.is_none());
        assert!(h.query_all_by_label_contains("Weekstart").next().is_none(), "the skipped time is gone from this week");
        let s = &h.state().engine.session;
        assert_eq!(s.appointment(&weekly.id).unwrap().recurrence.as_ref().unwrap().skipped, vec![clicked]);
        assert_eq!(s.appointments.iter().filter(|a| a.title == "Weekstart").count(), 1, "the series itself stays");

        // A new appointment can be made to repeat; the file then holds the rule, not the copies.
        h.get_by_label("Nieuwe afspraak").click();
        h.run();
        {
            let e = h.state_mut().calendar.editor.as_mut().expect("editor");
            e.title = "Yoga".into();
            e.repeat = Some(mailcraft_model::Frequency::Daily);
            e.ends = calendar::Ends::AfterCount;
            e.count = "3".into();
        }
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        let s = &h.state().engine.session;
        let yoga = s.appointments.iter().find(|a| a.title == "Yoga").expect("saved");
        assert_eq!(yoga.recurrence.as_ref().map(|r| r.count), Some(Some(3)));
        assert_eq!(s.appointments_between(yoga.start, yoga.start + 30 * 86_400).iter().filter(|a| a.title == "Yoga").count(), 3);
    }

    #[test]
    fn a_due_reminder_pops_up_and_can_be_closed() {
        let mut h = harness();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        h.run();
        assert!(h.query_by_label_contains("Herinnering").is_none(), "the sample calendar reminds of nothing");
        let start = now + 600;
        h.state_mut()
            .engine
            .run(
                "appointment.save",
                &serde_json::json!({ "title": "Tandarts", "location": "Praktijk", "start": start, "end": start + 1800, "reminder": 15 }),
            )
            .expect("saved");
        h.run();
        h.get_by_label_contains("Tandarts");
        assert!(h.query_by_label_contains("Begint over 10 minuten").is_some() || h.query_by_label_contains("Begint over 9 minuten").is_some());
        assert_eq!(h.state().announced_reminders.len(), 1);
        h.get_by_label("Openen").click();
        h.run();
        assert_eq!(h.state().module, Module::Calendar);
        assert_eq!(h.state().calendar.editor.as_ref().map(|e| e.title.as_str()), Some("Tandarts"));
        h.get_by_label("Annuleren").click();
        h.run();
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.query_by_label_contains("Begint over").is_none(), "closed");
        let s = &h.state().engine.session;
        assert_eq!(s.dismissed_reminders.len(), 1);
        assert!(h.state().announced_reminders.is_empty());
    }

    #[test]
    fn a_rule_is_made_in_the_window_and_run_over_the_open_folder() {
        let mut h = harness();
        h.run();
        h.get_by_label("Regels").click();
        h.run();
        assert!(h.state().rules.is_some());
        h.get_by_label("Nog geen regels.");
        h.get_by_label("Nieuwe regel").click();
        h.run();
        h.get_by_label("Regel opslaan").click();
        h.run();
        let form = h.state().rules.as_ref().and_then(|s| s.form.as_ref()).expect("the form stays open on an error");
        assert_eq!(form.error.as_deref(), Some("Vul in waar de regel op moet letten."));
        {
            let form = h.state_mut().rules.as_mut().and_then(|s| s.form.as_mut()).expect("form");
            form.name = "Studio".into();
            form.contains = "STUDIO@".into();
            form.move_to = Some("archive".into());
            form.mark_read = true;
            form.categories = vec!["Rode categorie".into()];
        }
        h.run();
        h.get_by_label("Regel opslaan").click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.rules.len(), 1);
        assert_eq!((s.rules[0].name.as_str(), s.rules[0].contains.as_str(), s.rules[0].account_id.as_str()), ("Studio", "STUDIO@", "sample"));
        assert!(h.state().rules.as_ref().is_some_and(|r| r.form.is_none()));
        h.get_by_label_contains("Van bevat “STUDIO@” → naar Archief, markeren als gelezen, categorie Rode categorie");
        assert_eq!(s.message("m0").unwrap().folder_id, "inbox", "saving a rule moves nothing by itself");
        h.get_by_label("Regels nu uitvoeren op deze map").click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.message("m0").unwrap().folder_id, "archive");
        assert!(s.message("m0").unwrap().flags.seen);
        assert_eq!(s.message("m0").unwrap().categories, vec!["Rode categorie".to_string()]);
        assert_eq!(h.state().notice.as_deref(), Some("Regels toegepast op 1 bericht."));
        h.get_by_label("Regel verwijderen").click();
        h.run();
        assert!(h.state().engine.session.rules.is_empty());
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().rules.is_none());
    }

    #[test]
    fn a_message_is_moved_to_a_chosen_folder_from_the_ribbon() {
        let mut h = harness();
        h.run();
        h.get_by_label("Welkom bij Mailcraft").click();
        h.run();
        let id = h.state().engine.session.selected_message.clone().expect("selected");
        h.get_by_label("Verplaatsen").click();
        h.run();
        assert!(h.state().move_to);
        assert!(h.query_by_label("📥 Postvak IN").is_none(), "the folder it is in is not offered");
        h.get_by_label("🗄 Archief").click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.message(&id).unwrap().folder_id, "archive");
        assert!(s.selected_message.is_none() && !h.state().move_to);
        assert!(h.query_by_label("Welkom bij Mailcraft").is_none(), "gone from the inbox list");
        h.get_by_label("Archief").click();
        h.run();
        h.get_by_label("Welkom bij Mailcraft");
    }

    #[test]
    fn a_folder_is_made_renamed_and_removed_from_the_map_tab() {
        let mut h = harness();
        h.run();
        h.get_by_label("Map").click();
        h.run();
        h.get_by_label("Nieuwe map").click();
        h.run();
        h.get_by_label("Map aanmaken").click();
        h.run();
        let edit = h.state().folder_edit.as_ref().expect("the window stays open on an error");
        assert!(edit.error.as_deref().is_some_and(|e| e.starts_with("Geef de map een naam")), "{:?}", edit.error);
        h.state_mut().folder_edit.as_mut().expect("edit").name = "Projecten".into();
        h.run();
        h.get_by_label("Map aanmaken").click();
        h.run();
        assert!(h.state().folder_edit.is_none());
        h.get_by_label("Projecten").click();
        h.run();
        assert_eq!(h.state().engine.session.selected_folder.as_deref(), Some("sample/INBOX/Projecten"));

        h.get_by_label("Naam wijzigen").click();
        h.run();
        assert_eq!(h.state().folder_edit.as_ref().map(|e| e.name.as_str()), Some("Projecten"));
        h.state_mut().folder_edit.as_mut().expect("edit").name = "Klanten".into();
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        assert!(h.query_all_by_label("Projecten").next().is_none());
        assert!(h.query_all_by_label("Klanten").next().is_some(), "the folder row carries the new name");
        assert_eq!(h.state().engine.session.selected_folder.as_deref(), Some("sample/INBOX/Klanten"));

        h.get_by_label("Map verwijderen").click();
        h.run();
        h.get_by_label("Ja, verwijderen").click();
        h.run();
        assert!(h.query_all_by_label("Klanten").next().is_none());
        assert_eq!(h.state().engine.session.selected_folder.as_deref(), Some("inbox"), "back to the inbox");
        // The inbox belongs to the account: the buttons are off and the menu says so.
        h.get_by_label("Map verwijderen");
        h.state_mut().delete_folder("inbox".into());
        h.run();
        assert!(h.state().folder_edit.is_none());
        assert!(h.state().notice.as_deref().is_some_and(|n| n.contains("hoort bij het account")));
    }

    #[test]
    fn deleting_from_the_deleted_folder_asks_first_and_a_folder_can_be_emptied_or_marked_read() {
        let mut h = harness();
        h.run();
        h.get_by_label("Map").click();
        h.run();
        assert!(h.state().engine.session.folder("inbox").is_some_and(|f| f.unread > 0));
        h.get_by_label("Alles gelezen").click();
        h.run();
        assert_eq!(h.state().engine.session.folder("inbox").map(|f| f.unread), Some(0));

        // Shortcuts belong to the Start tab.
        h.get_by_label("Start").click();
        h.run();
        h.get_by_label("Welkom bij Mailcraft").click();
        h.run();
        h.key_press(egui::Key::Delete);
        h.run();
        assert_eq!(h.state().engine.session.message("m0").map(|m| m.folder_id.as_str()), Some("deleted"), "first to the deleted folder");
        h.get_by_label("Verwijderde items").click();
        h.run();
        h.get_by_label("Welkom bij Mailcraft").click();
        h.run();
        h.key_press(egui::Key::Delete);
        h.run();
        assert!(h.state().engine.session.message("m0").is_some(), "nothing goes before the question is answered");
        assert!(matches!(h.state().folder_edit.as_ref().map(|e| &e.action), Some(folders::Action::DeleteForever { .. })));
        h.get_by_label("Annuleren").click();
        h.run();
        assert!(h.state().folder_edit.is_none() && h.state().engine.session.message("m0").is_some());
        h.get_by_label("Verwijderen").click();
        h.run();
        h.get_by_label("Ja, verwijderen").click();
        h.run();
        assert!(h.state().engine.session.message("m0").is_none());

        h.get_by_label("Postvak IN").click();
        h.run();
        h.get_by_label("Map").click();
        h.run();
        h.get_by_label("Map leegmaken").click();
        h.run();
        h.get_by_label("Ja, leegmaken").click();
        h.run();
        let s = &h.state().engine.session;
        assert!(s.messages.iter().all(|m| m.folder_id != "inbox"));
        assert!(h.query_all_by_label("Planning voor donderdag").next().is_none());
    }

    #[test]
    fn the_view_tab_filters_the_list_moves_the_reading_pane_and_sets_the_theme() {
        let mut h = harness();
        h.run();
        h.get_by_label("Beeld").click();
        h.run();
        h.get_by_value("Alle").click();
        h.run();
        h.get_by_label("Groene categorie").click();
        h.run();
        assert_eq!(h.state().engine.session.category_filter.as_deref(), Some("Groene categorie"));
        assert!(h.query_all_by_label("Welkom bij Mailcraft").next().is_none(), "only mail with the category is listed");
        h.get_by_label("Planning voor donderdag");
        h.get_by_value("Groene categorie").click();
        h.run();
        h.get_by_label("Alle").click();
        h.run();
        assert!(h.state().engine.session.category_filter.is_none());
        h.get_by_label("Ongelezen").click();
        h.run();
        assert!(h.query_all_by_label("Re: Offerte drukwerk").next().is_none(), "read mail is hidden");
        h.get_by_label("Welkom bij Mailcraft");
        h.get_by_label("Met bijlage").click();
        h.run();
        assert!(h.query_all_by_label("Welkom bij Mailcraft").next().is_none());
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        h.get_by_label("Alles").click();
        h.run();
        assert_eq!(h.state().engine.session.visible_messages().len(), 4);

        h.get_by_label("Onder").click();
        h.run();
        assert_eq!(h.state().view.reading_pane, view::ReadingPane::Bottom);
        h.get_by_label("Welkom bij Mailcraft").click();
        h.run();
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some("m0"), "the list still works under the reading pane");
        h.get_by_label("Compact").click();
        h.run();
        assert!(h.state().view.compact);

        h.get_by_label("Donker").click();
        h.run();
        h.run();
        assert_eq!(h.state().view.theme, view::ThemeChoice::Dark);
        assert_eq!(h.ctx.theme(), egui::Theme::Dark);
        h.get_by_label("Licht").click();
        h.run();
        h.run();
        assert_eq!(h.ctx.theme(), egui::Theme::Light);
    }

    #[test]
    fn a_quick_step_is_made_in_the_window_and_run_from_the_ribbon_and_the_keyboard() {
        use egui::{Key, Modifiers};
        let mut h = harness();
        h.run();
        h.get_by_label("Beheren…").click();
        h.run();
        assert!(h.state().quick_steps.is_some());
        h.get_by_label("Nog geen snelle stappen.");
        h.get_by_label("Nieuwe snelle stap").click();
        h.run();
        h.get_by_label("Snelle stap opslaan").click();
        h.run();
        let form = h.state().quick_steps.as_ref().and_then(|s| s.form.as_ref()).expect("the form stays open on an error");
        assert_eq!(form.error.as_deref(), Some("Geef de snelle stap een naam van hoogstens 40 tekens."));
        {
            let form = h.state_mut().quick_steps.as_mut().and_then(|s| s.form.as_mut()).expect("form");
            form.name = "Klaar".into();
            form.move_to = Some("archive".into());
            form.mark_read = true;
        }
        h.run();
        h.get_by_label("Snelle stap opslaan").click();
        h.run();
        assert_eq!(h.state().engine.session.quick_steps.len(), 1);
        h.get_by_label("naar Archief, markeren als gelezen");
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().quick_steps.is_none());

        h.get_by_label("Welkom bij Mailcraft").click();
        h.run();
        h.get_by_label("⚡ Klaar").click();
        h.run();
        let m0 = h.state().engine.session.message("m0").cloned().expect("m0");
        assert!(m0.folder_id == "archive" && m0.flags.seen, "the step moved and marked the message");

        h.get_by_label("Planning voor donderdag").click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Num1);
        h.run();
        assert_eq!(
            h.state().engine.session.message("m1").map(|m| m.folder_id.clone()),
            Some("archive".to_string()),
            "the first step answers to the shortcut"
        );
    }

    #[test]
    fn the_help_tab_lists_the_shortcuts_and_tells_about_the_app() {
        let mut h = harness();
        h.run();
        h.get_by_label("Help").click();
        h.run();
        h.get_by_label("Sneltoetsen").click();
        h.run();
        assert_eq!(h.state().help, Some(help::HelpWindow::Shortcuts));
        h.get_by_label("Allen beantwoorden");
        h.get_by_label_contains("Shift+V");
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().help.is_none());
        h.get_by_label("Over Mailcraft").click();
        h.run();
        h.get_by_label(format!("Versie {}", env!("CARGO_PKG_VERSION")).as_str());
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().help.is_none());
    }

    #[test]
    fn a_category_is_put_on_a_message_from_the_menu_and_the_list_is_managed() {
        let mut h = harness();
        h.run();
        h.get_by_label("Welkom bij Mailcraft").click();
        h.run();
        h.get_by_label_contains("Categoriseren").click();
        h.run();
        h.get_by_label_contains("Rode categorie").click();
        h.run();
        assert_eq!(h.state().engine.session.message("m0").map(|m| m.categories.clone()), Some(vec!["Rode categorie".to_string()]));
        // The chip above the message takes it off again.
        h.get_by_label("Rode categorie").click();
        h.run();
        assert!(h.state().engine.session.message("m0").is_some_and(|m| m.categories.is_empty()));

        h.state_mut().open_categories();
        h.run();
        h.state_mut().categories.as_mut().expect("manager").name = "Klanten".into();
        h.run();
        h.get_by_label("Toevoegen").click();
        h.run();
        assert!(h.state().engine.session.categories.iter().any(|c| c.name == "Klanten"));
        assert_eq!(h.state().categories.as_ref().map(|m| m.name.as_str()), Some(""), "the field is cleared for the next one");
        h.get_by_label("Toevoegen").click();
        h.run();
        assert!(h.state().categories.as_ref().and_then(|m| m.error.as_deref()).is_some(), "an empty name is refused");
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().categories.is_none());
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
