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
pub mod cleanup;
mod compose;
pub mod contacts;
mod dates;
pub mod folders;
pub mod groups;
pub mod help;
mod html;
pub mod importance;
pub mod invitation;
pub mod junk;
pub mod meeting;
pub mod message_menu;
pub mod print;
pub mod quicksteps;
pub mod reminders;
pub mod remote;
pub mod rules;
pub mod search_folders;
pub mod tasks;
pub mod templates;
mod theme;
pub mod unsubscribe;
pub mod view;

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};

use mailcraft_engine::{AccountStatus, Engine};
use mailcraft_model::{FolderKind, Warning};

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
    /// The "Ongewenste e-mail" window, while open.
    pub junk: Option<junk::Options>,
    /// The quick steps window, when it is open.
    pub quick_steps: Option<quicksteps::State>,
    /// The "Sjablonen" window, when it is open.
    pub templates: Option<templates::State>,
    /// Whether the "Verplaatsen naar" window for the selected message is open.
    pub move_to: bool,
    /// The window for a new, renamed or removed folder, while it is open.
    pub folder_edit: Option<folders::FolderEdit>,
    /// The "Afmelden" question, while it is open.
    pub unsubscribe: Option<unsubscribe::Confirm>,
    pub cleanup: Option<cleanup::Confirm>,
    /// The picture attachment open in its preview window.
    pub preview: Option<attachments::Preview>,
    /// The selected message is open in its own window (Enter, a double-click, or always when the
    /// reading pane is off).
    pub message_window: bool,
    /// The form for saving the open search as a search folder, while open.
    pub search_folder_form: Option<search_folders::Form>,
    /// The "Alle categorieën" window, while it is open.
    pub categories: Option<categories::Manager>,
    /// The open help window ("Help" tab), if any.
    pub help: Option<help::HelpWindow>,
    /// What the "Beeld" tab chose; read from `view.json` at start.
    pub view: view::ViewPrefs,
    /// The conversations (by key) unfolded in the list, while the conversation view is on.
    pub expanded: std::collections::HashSet<String>,
    /// Whether the saved theme has been handed to egui yet (on the first frame).
    theme_applied: bool,
    /// The window title last handed to egui, so it is only sent when it changes.
    title: Option<String>,
}

/// The window title: the app's name, with the unread inbox mail of every account in brackets
/// when there is any, as Outlook shows it.
pub fn window_title(inbox_unread: u32) -> String {
    if inbox_unread == 0 { "Mailcraft".to_string() } else { format!("Mailcraft ({inbox_unread})") }
}

/// A message being dragged from the list, to drop on a folder.
struct DraggedMessage(String);

/// What the reading pane says while several messages are selected.
pub(crate) const MULTI_HINT: &str = "Verwijderen, Archiveren, Ongewenst, Verplaatsen, Gelezen of Ongelezen en Vlag gelden voor alle geselecteerde berichten. Klik zonder toets om er weer één te openen.";

/// One row of the message list: a message, indented when it sits inside an unfolded
/// conversation, with the fold control when it heads a conversation of several messages.
struct Row<'a> {
    m: &'a mailcraft_model::MessageHeader,
    indent: bool,
    fold: Option<Fold>,
}

/// The fold control of a conversation head: how many messages it hides and whether it is open.
struct Fold {
    key: String,
    count: usize,
    open: bool,
}

/// A heading over the messages of one date group, in the band Outlook uses.
fn group_heading(ui: &mut Ui, t: &Tokens, group: &str) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 24.0));
    ui.painter().rect_filled(rect, CornerRadius::ZERO, t.panel);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.label(RichText::new(group).size(12.0).strong().color(t.text_dim));
    });
}

/// The rows of the list: every message on its own when the conversation view is off; else one
/// row per conversation, followed by its other messages when it is unfolded. A conversation that
/// holds the selected message is shown unfolded for as long as it does, so a message reached
/// with the arrow keys is seen.
fn list_rows<'a>(
    list: Vec<&'a mailcraft_model::MessageHeader>,
    conversations: bool,
    expanded: &std::collections::HashSet<String>,
    selected: Option<&str>,
) -> Vec<Row<'a>> {
    if !conversations {
        return list.into_iter().map(|m| Row { m, indent: false, fold: None }).collect();
    }
    let mut rows = Vec::new();
    for c in mailcraft_engine::view::conversations(list) {
        let (Some(head), rest) = (c.messages.first().copied(), &c.messages[1..]) else { continue };
        if rest.is_empty() {
            rows.push(Row { m: head, indent: false, fold: None });
            continue;
        }
        let open = expanded.contains(&c.key) || rest.iter().any(|m| Some(m.id.as_str()) == selected);
        rows.push(Row { m: head, indent: false, fold: Some(Fold { key: c.key.clone(), count: c.messages.len(), open }) });
        if open {
            rows.extend(rest.iter().map(|m| Row { m, indent: true, fold: None }));
        }
    }
    rows
}

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
            unsubscribe: None,
            cleanup: None,
            preview: None,
            message_window: false,
            search_folder_form: None,
            shown_message: None,
            remote: remote::RemoteImages::default(),
            drafts_for: None,
            announced_reminders: std::collections::HashSet::new(),
            rules: None,
            junk: None,
            quick_steps: None,
            templates: None,
            move_to: false,
            folder_edit: None,
            categories: None,
            help: None,
            view,
            expanded: std::collections::HashSet::new(),
            theme_applied: false,
            title: None,
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
        if let Some(failure) = self.engine.session.take_scheduled_failures().pop() {
            self.notice = Some(failure);
        }
        let title = window_title(self.engine.session.inbox_unread());
        if self.title.as_deref() != Some(title.as_str()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = Some(title);
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
        groups::window(self, &ctx, &t);
        reminders::window(self, &ctx, &t);
        rules::window(self, &ctx, &t);
        junk::window(self, &ctx, &t);
        quicksteps::window(self, &ctx, &t);
        templates::window(self, &ctx, &t);
        move_window(self, &ctx, &t);
        folders::window(self, &ctx, &t);
        unsubscribe::window(self, &ctx, &t);
        cleanup::window(self, &ctx, &t);
        attachments::preview_window(self, &ctx, &t);
        message_window(self, &ctx, &t);
        search_folders::window(self, &ctx, &t);
        categories::window(self, &ctx, &t);
        help::window(self, &ctx, &t);
        drag_preview(self, &ctx, &t);
        calendar::drag_preview(self, &ctx, &t);
        if self.tab == Tab::Home && self.module == Module::Mail {
            shortcuts(self, &ctx);
        } else if self.tab == Tab::Home {
            module_shortcuts(self, &ctx);
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
                view::ReadingPane::Off => {
                    // Outlook's "Leesvenster uit": the list takes the room and a message is read
                    // in its own window.
                    egui::CentralPanel::default().frame(frame).show(ui, |ui| message_list(self, ui, &t));
                }
            }
            if self.view.reading_pane != view::ReadingPane::Off {
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reading).inner_margin(Margin::same(20))).show(ui, |ui| {
                    reading_pane(self, ui, &t);
                });
            }
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

/// The two search keys in the calendar, contacts and tasks modules: Ctrl+E (Cmd+E on a Mac)
/// puts the cursor in the module's search box, and Esc, once no text field has the keyboard,
/// clears it.
fn module_shortcuts(app: &mut MailApp, ctx: &egui::Context) {
    use egui::{Key, Modifiers};
    if app.compose.is_some() {
        return;
    }
    if !ctx.egui_wants_keyboard_input() && new_item_shortcuts(app, ctx) {
        return;
    }
    if app.module == Module::Calendar && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::P)) {
        app.print_calendar();
        return;
    }
    let (box_id, query) = match app.module {
        Module::Mail => return,
        Module::Calendar => (calendar::SEARCH_BOX, &mut app.calendar.query),
        Module::People => (contacts::SEARCH_BOX, &mut app.contacts.query),
        Module::Tasks => (tasks::SEARCH_BOX, &mut app.tasks.query),
    };
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::E)) {
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(box_id)));
    } else if !ctx.egui_wants_keyboard_input() && !query.trim().is_empty() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        query.clear();
    }
}

/// Outlook's keys for a new item, from every module: Ctrl+Shift+M a message, Ctrl+Shift+A an
/// appointment (the next full hour of today), Ctrl+Shift+C a contact and Ctrl+Shift+K a task
/// (Cmd on a Mac). The contact and task forms live in their module, so those two switch to it.
/// Returns whether one fired.
fn new_item_shortcuts(app: &mut MailApp, ctx: &egui::Context) -> bool {
    use chrono::Timelike;
    use egui::{Key, Modifiers};
    let keys = Modifiers::COMMAND | Modifiers::SHIFT;
    if ctx.input_mut(|i| i.consume_key(keys, Key::M)) {
        app.open_compose(ComposeKind::New);
    } else if ctx.input_mut(|i| i.consume_key(keys, Key::A)) {
        let now = chrono::Local::now();
        app.new_appointment(now.date_naive(), (now.hour() + 1).min(23));
    } else if ctx.input_mut(|i| i.consume_key(keys, Key::C)) {
        app.module = Module::People;
        app.new_contact();
    } else if ctx.input_mut(|i| i.consume_key(keys, Key::K)) {
        app.module = Module::Tasks;
        app.new_task();
    } else {
        return false;
    }
    true
}

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
    if new_item_shortcuts(app, ctx) {
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
        } else if i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::R) {
            command = Some("meeting");
        } else if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::R) {
            compose = Some(ComposeKind::ReplyAll);
        } else if i.consume_key(Modifiers::COMMAND, Key::R) {
            compose = Some(ComposeKind::Reply);
        } else if i.consume_key(Modifiers::COMMAND, Key::F) {
            compose = Some(ComposeKind::Forward);
        } else if i.consume_key(Modifiers::COMMAND, Key::P) {
            command = Some("message.print");
        } else if i.consume_key(Modifiers::CTRL, Key::Delete) {
            command = Some("conversation.ignore");
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
        } else if i.consume_key(Modifiers::COMMAND, Key::A) {
            command = Some("message.select_all");
        } else if i.consume_key(Modifiers::NONE, Key::F5) {
            command = Some("folder.refresh");
        } else if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            step = 1;
        } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            step = -1;
        } else if i.consume_key(Modifiers::NONE, Key::Enter) {
            command = Some("message.open_window");
        } else if i.consume_key(Modifiers::NONE, Key::Escape) && (app.message_window || app.engine.session.is_searching()) {
            command = Some("escape");
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
        Some("escape") if app.message_window => app.message_window = false,
        Some("escape") => app.engine.session.search.clear(),
        Some("message.open_window") if has => app.message_window = true,
        Some("folder.refresh") => app.run("folder.refresh", Value::Null),
        Some("message.select_all") => {
            let ids: Vec<String> = app.engine.session.visible_messages().iter().map(|m| m.id.clone()).collect();
            if !ids.is_empty() {
                app.run("message.select_many", json!({ "ids": ids }));
            }
        }
        Some("message.move_window") if has => app.move_to = true,
        Some("meeting") => {
            if let Some(m) = app.engine.session.selected().cloned() {
                app.meeting_from_message(&m);
            }
        }
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
    if ribbon_action_hint(ui, t, "📅", "Nieuwe afspraak", true, Some(&key_hint("Shift+A"))) {
        let date = app.calendar.focus;
        app.new_appointment(date, 9);
    }
    ui.separator();
    if ribbon_action(ui, t, "◎", "Vandaag", true) {
        app.calendar.focus = calendar::today();
    }
    for (glyph, label, view) in [("▯", "Dag", View::Day), ("▤", "Werkweek", View::WorkWeek), ("▥", "Week", View::Week), ("▦", "Maand", View::Month)]
    {
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

/// One row of the folder pane: icon, name and the unread count, highlighted when it is the
/// open folder. `hint` is shown on hover, when there is one.
fn folder_row(ui: &mut Ui, t: &Tokens, f: &mailcraft_model::Folder, on: bool, hint: Option<String>) -> egui::Response {
    let r = ui.add_sized(
        [ui.available_width(), 28.0],
        egui::Button::new("").fill(if on { t.checked } else { Color32::TRANSPARENT }).frame_when_inactive(on),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &f.name));
    let r = match hint {
        Some(h) => r.on_hover_text(h),
        None => r,
    };
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
    r
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
            // Outlook's "Favorieten": the pinned folders of every account, above the accounts.
            let favorites: Vec<mailcraft_model::Folder> = app.view.favorites.iter().filter_map(|id| app.engine.session.folder(id).cloned()).collect();
            if !favorites.is_empty() {
                ui.label(RichText::new("★ Favorieten").strong().color(t.text));
                ui.add_space(4.0);
                for f in &favorites {
                    let on = app.engine.session.selected_folder.as_deref() == Some(f.id.as_str());
                    let account = app.engine.session.accounts.iter().find(|a| a.id == f.account_id).map(|a| a.address.email().to_string());
                    let r = folder_row(ui, t, f, on, account.map(|a| format!("{} van {a}", f.name)));
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
                ui.add_space(12.0);
            }
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
                    let r = folder_row(ui, t, f, on, None);
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
            // Outlook's "Zoekmappen": the saved searches, under the accounts.
            search_folders::section(app, ui, t);
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
            RichText::new(
                "Berichten die je opzij hebt gezet, en berichten die later verzonden worden. Ze staan alleen op deze computer, in de versleutelde offline kopie.",
            )
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match d.send_at {
                        Some(ts) => {
                            ui.label(RichText::new(format!("⏰ Verzenden {}", compose::when_text(ts))).size(12.0).color(t.accent_text))
                                .on_hover_text("Gaat op dat moment vanzelf weg, als Mailcraft dan draait. Open het concept om dat te veranderen.");
                        }
                        None => {
                            ui.label(RichText::new(dates::short(d.updated)).size(12.0).color(t.text_dim));
                        }
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
            let mut save_search = false;
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
                    let width = (ui.available_width() - if searching { 240.0 } else { 150.0 }).max(80.0);
                    ui.add(egui::TextEdit::singleline(&mut session.search).id(egui::Id::new(SEARCH_BOX)).hint_text(hint).desired_width(width));
                    if searching && ui.small_button("✕").on_hover_text("Zoekopdracht wissen").clicked() {
                        session.search.clear();
                    }
                    if searching && ui.small_button("Zoekmap").on_hover_text("Deze zoekopdracht bewaren als zoekmap in het mappenvenster").clicked() {
                        save_search = true;
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
            if save_search {
                app.ask_search_folder();
            }
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
                let selected = app.engine.session.selected_message.clone();
                let rows = list_rows(list, app.view.conversations, &app.expanded, selected.as_deref());
                let row_ids: Vec<String> = rows.iter().map(|r| r.m.id.clone()).collect();
                let mut toggle: Option<String> = None;
                let mut menu_pick: Option<(mailcraft_model::MessageHeader, message_menu::Pick)> = None;
                let mut open_window: Option<String> = None;
                // Date groups, as Outlook heads the list, while the list is in date order.
                let grouped =
                    app.view.groups && matches!(app.engine.session.sort, mailcraft_engine::view::Sort::Newest | mailcraft_engine::view::Sort::Oldest);
                let today = chrono::Local::now().date_naive();
                let mut last_group: Option<&str> = None;
                for row in rows {
                    let m = row.m;
                    if grouped && !row.indent {
                        let group = dates::date_group(m.date, today);
                        if last_group != Some(group) {
                            last_group = Some(group);
                            group_heading(ui, t, group);
                        }
                    }
                    let on = selected.as_deref() == Some(m.id.as_str()) || app.engine.session.marked.contains(&m.id);
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click_and_drag());
                    r.dnd_set_drag_payload(DraggedMessage(m.id.clone()));
                    let hover = match app.engine.session.follow_up(&m.id).and_then(mailcraft_engine::followup::due_day) {
                        Some(d) => format!("{}\nOpvolgen vóór {}", m.subject, dates::day_name(d)),
                        None => m.subject.clone(),
                    };
                    let r = r.on_hover_text(hover);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &m.subject));
                    if r.double_clicked() {
                        open_window = Some(m.id.clone());
                    }
                    let known_sender = app.engine.session.contact_with_email(m.from.email()).is_some();
                    let in_junk = app.engine.session.folder(&m.folder_id).is_some_and(|f| f.kind == FolderKind::Junk);
                    let ignored = app.engine.session.message_ignored(&m.id);
                    if let Some(p) = message_menu::menu(&r, m, known_sender, in_junk, ignored) {
                        menu_pick = Some((m.clone(), p));
                    }
                    // The fold control is its own widget, drawn over the row's left edge.
                    if let Some(fold) = &row.fold {
                        let box_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 16.0, rect.top() + 14.0), vec2(18.0, 18.0));
                        let c = ui.interact(box_rect, ui.make_persistent_id(("fold", &m.id)), Sense::click());
                        let label = if fold.open { "Gesprek inklappen" } else { "Gesprek uitvouwen" };
                        c.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
                        if c.clicked() {
                            toggle = Some(fold.key.clone());
                        }
                    }
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
                    let shift = if row.indent {
                        32.0
                    } else if row.fold.is_some() {
                        16.0
                    } else {
                        0.0
                    };
                    let x = rect.left() + 14.0 + shift;
                    let w = rect.width() - 28.0 - shift;
                    let from_color = if m.flags.seen { t.text } else { t.accent_text };
                    let mut from = clip(m.from.display(), 34);
                    if let Some(fold) = &row.fold {
                        let glyph = if fold.open { "▾" } else { "▸" };
                        p.text(
                            egui::pos2(rect.left() + 16.0, rect.top() + 14.0),
                            egui::Align2::CENTER_CENTER,
                            glyph,
                            egui::FontId::proportional(14.0),
                            t.text_dim,
                        );
                        from = format!("{} ({})", clip(m.from.display(), 28), fold.count);
                    }
                    p.text(egui::pos2(x, rect.top() + 14.0), egui::Align2::LEFT_CENTER, from, egui::FontId::proportional(14.0), from_color);
                    let mut right = dates::short(m.date);
                    if m.has_attachments {
                        right = format!("📎 {right}");
                    }
                    if m.flags.flagged {
                        right = format!("⚑ {right}");
                    }
                    let right_rect = p.text(
                        egui::pos2(x + w, rect.top() + 14.0),
                        egui::Align2::RIGHT_CENTER,
                        right,
                        egui::FontId::proportional(12.0),
                        if m.flags.flagged { t.flag } else { t.text_dim },
                    );
                    if let Some((glyph, color)) = importance::mark(m.importance, t) {
                        p.text(
                            egui::pos2(right_rect.left() - 6.0, rect.top() + 14.0),
                            egui::Align2::RIGHT_CENTER,
                            glyph,
                            egui::FontId::proportional(15.0),
                            color,
                        );
                    }
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
                        pick = Some((m.id.clone(), click_modifiers(ui), row_ids.clone()));
                    }
                }
                if let Some(key) = toggle
                    && !app.expanded.remove(&key)
                {
                    app.expanded.insert(key);
                }
                if let Some((m, p)) = menu_pick {
                    app.act_on_message(&m, p);
                }
                if let Some(id) = open_window {
                    if app.engine.session.selected_message.as_deref() != Some(id.as_str()) || !app.engine.session.marked.is_empty() {
                        app.run("message.select", json!({ "id": id }));
                    }
                    app.message_window = true;
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
            if let Some((id, mods, row_ids)) = pick {
                select_row(app, &row_ids, id, mods);
            }
            if older {
                app.run("folder.older", Value::Null);
            }
        }
    }
}

impl MailApp {
    /// Shows a message in the mail module: its folder opens and it is selected, from wherever
    /// the person was (the flagged mail in the tasks module, for one).
    pub fn open_message(&mut self, id: &str) {
        let Some(folder) = self.engine.session.message(id).map(|m| m.folder_id.clone()) else { return };
        self.module = Module::Mail;
        self.drafts_for = None;
        if self.engine.session.selected_folder.as_deref() != Some(folder.as_str()) {
            self.run("folder.select", json!({ "id": folder }));
        }
        self.run("message.select", json!({ "id": id }));
    }
}

/// The modifiers held during this frame's click: those on the button-up event, which is what
/// makes a click, else the ones held now.
fn click_modifiers(ui: &Ui) -> egui::Modifiers {
    ui.input(|i| {
        i.events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(i.modifiers)
    })
}

/// A click on a row of the list: plain selects the message, Ctrl (Cmd on a Mac) adds it to or
/// takes it out of the selection, Shift selects everything from the selected message to it.
fn select_row(app: &mut MailApp, row_ids: &[String], id: String, mods: egui::Modifiers) {
    let anchor = app.engine.session.selected_message.as_deref().and_then(|s| row_ids.iter().position(|r| r == s));
    let clicked = row_ids.iter().position(|r| *r == id);
    if mods.shift
        && let (Some(a), Some(c)) = (anchor, clicked)
    {
        let (from, to) = if a <= c { (a, c) } else { (c, a) };
        let mut ids: Vec<String> = row_ids[from..=to].to_vec();
        // The selected message stays where it is: first in the list, so it keeps the reading pane.
        if let Some(sel) = app.engine.session.selected_message.clone() {
            ids.retain(|r| *r != sel);
            ids.insert(0, sel);
        }
        app.run("message.select_many", json!({ "ids": ids }));
    } else if mods.command {
        app.run("message.select_more", json!({ "id": id }));
    } else {
        app.run("message.select", json!({ "id": id }));
    }
}

/// The selected message in its own window, as a double-click opens it in Outlook: the same
/// reading pane in a movable window; Escape or its cross closes it. It closes by itself when
/// nothing is selected any more.
fn message_window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    if !app.message_window {
        return;
    }
    let Some(subject) = app.engine.session.selected().map(|m| m.subject.clone()) else {
        app.message_window = false;
        return;
    };
    let mut open = true;
    egui::Window::new(format!("✉ {subject}"))
        .id(egui::Id::new("message-window"))
        .open(&mut open)
        .default_size(vec2(760.0, 560.0))
        .resizable(true)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.reading).inner_margin(Margin::same(16)))
        .show(ctx, |ui| reading_pane(app, ui, t));
    if !open {
        app.message_window = false;
    }
}

fn reading_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(m) = app.engine.session.selected().cloned() else {
        ui.centered_and_justified(|ui| ui.label(RichText::new("Selecteer een bericht om het te lezen.").color(t.text_dim).size(15.0)));
        return;
    };
    if !app.engine.session.marked.is_empty() {
        // Several messages are selected: the pane says so, as Outlook does, and the ribbon's
        // actions take them all.
        let n = app.engine.session.marked.len() + 1;
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.label(RichText::new(format!("{n} berichten geselecteerd")).size(18.0).strong().color(t.text));
            ui.add_space(8.0);
            ui.label(RichText::new(MULTI_HINT).color(t.text_dim));
        });
        return;
    }
    ui.label(RichText::new(&m.subject).size(20.0).strong().color(t.text));
    ui.add_space(10.0);
    importance::strip(ui, t, m.importance);
    if m.flags.flagged {
        // The follow-up strip, as Outlook puts it over a flagged message: the due day, red once
        // it has passed, and the way to mark it done.
        let today = chrono::Local::now().date_naive();
        let due = app.engine.session.follow_up(&m.id).and_then(mailcraft_engine::followup::due_day);
        let (text, late) = match due {
            Some(d) if d < today => (format!("⚑ Opvolgen: verlopen op {}", dates::day_name(d)), true),
            Some(d) if d == today => ("⚑ Opvolgen: vandaag".to_string(), false),
            Some(d) => (format!("⚑ Opvolgen vóór {}", dates::day_name(d)), false),
            None => ("⚑ Gemarkeerd voor opvolging".to_string(), false),
        };
        let mut done = false;
        egui::Frame::new().fill(if late { t.flag.gamma_multiply(0.15) } else { t.checked }).corner_radius(4).inner_margin(8).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(text).color(if late { t.flag } else { t.text }));
                done = ui.small_button("Voltooid").on_hover_text("Haalt de vlag en de datum weg.").clicked();
            });
        });
        ui.add_space(8.0);
        if done {
            app.run("message.flag", Value::Null);
        }
    }
    let in_junk = app.engine.session.folder(&m.folder_id).is_some_and(|f| f.kind == FolderKind::Junk);
    if in_junk {
        // A message in the Junk folder: a warning, the links, attachments and pictures switched
        // off (as Outlook does), and the way back for a sender who is not junk.
        let mut not_junk = false;
        egui::Frame::new().fill(t.flag.gamma_multiply(0.15)).corner_radius(4).inner_margin(8).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(JUNK_NOTICE).color(t.text));
                not_junk = ui
                    .small_button("Geen ongewenst")
                    .on_hover_text(
                        "Zet het bericht terug in Postvak IN en maakt de afzender een veilige afzender; links en bijlagen werken dan weer.",
                    )
                    .clicked();
            });
        });
        ui.add_space(8.0);
        if not_junk {
            app.run("junk.trust", json!({ "id": m.id }));
            return;
        }
    }
    if app.engine.session.message_ignored(&m.id) {
        // An ignored conversation, as Outlook shows it: the way back is at hand.
        let mut stop = false;
        egui::Frame::new().fill(t.checked).corner_radius(4).inner_margin(8).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(IGNORED_NOTICE).color(t.text));
                stop = ui
                    .small_button("Niet meer negeren")
                    .on_hover_text("Zet de berichten van dit gesprek terug in Postvak IN; nieuwe berichten erin komen daar weer aan.")
                    .clicked();
            });
        });
        ui.add_space(8.0);
        if stop {
            app.run("conversation.unignore", json!({ "id": m.id }));
            return;
        }
    }
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
        let mut sender = false;
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.label(RichText::new(dates::long(m.date)).size(12.0).color(t.text_dim));
            let known = app.engine.session.contact_with_email(m.from.email()).is_some();
            let (label, hint) = if known {
                ("Contact openen", "Deze afzender staat in je contacten; opent de kaart.")
            } else {
                (
                    "Toevoegen aan contacten",
                    "Maakt een contact met de naam en het adres van deze afzender; je kunt het nog aanvullen voor je opslaat.",
                )
            };
            sender = ui.small_button(label).on_hover_text(hint).clicked();
        });
        if sender {
            app.open_or_add_sender(&m.from);
        }
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
    // Signs of phishing go above everything else, with the way to be rid of the sender.
    let warnings = message_warnings(app, &m, body.as_ref());
    if !warnings.is_empty() {
        let mut block = false;
        egui::Frame::NONE.fill(t.flag.gamma_multiply(0.18)).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 6)).show(
            ui,
            |ui| {
                ui.label(RichText::new(PHISHING_TITLE).strong().color(t.text));
                for w in &warnings {
                    ui.label(RichText::new(format!("• {}", w.text())).size(12.0).color(t.text));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Klik niet op links, open geen bijlagen en antwoord niet als je twijfelt.").size(12.0).color(t.text_dim));
                    block = ui
                        .small_button("Afzender blokkeren")
                        .on_hover_text(
                            "Zet het bericht in Ongewenste e-mail en blokkeert de afzender; volgende mail van dit adres gaat daar meteen heen.",
                        )
                        .clicked();
                });
            },
        );
        ui.add_space(8.0);
        if block {
            app.run("junk.block", json!({ "id": message_id }));
            return;
        }
    }
    // A list's mail offers the way off it, as Outlook does; not in the Junk folder, where the
    // header is not to be trusted.
    // A meeting invitation, as Outlook shows one over the message.
    if !in_junk && let Some(inv) = body.as_ref().and_then(|b| invitation::from_attachments(&b.attachments)) {
        let known = inv.appointment.as_ref().is_some_and(|a| app.has_appointment_like(a));
        if let Some(a) = invitation::strip(ui, t, &inv, known) {
            app.add_invitation(&a);
        }
        ui.add_space(8.0);
    }
    if !in_junk && let Some(targets) = body.as_ref().map(|b| b.unsubscribe.as_slice()).filter(|u| !u.is_empty()) {
        let mut ask = false;
        egui::Frame::new().fill(t.checked).corner_radius(4).inner_margin(8).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(unsubscribe::STRIP).color(t.text));
                ask = ui
                    .small_button("Afmelden…")
                    .on_hover_text("Vraagt eerst wat er gebeurt: een e-mail aan de afzender of een pagina in de browser.")
                    .clicked();
            });
        });
        ui.add_space(8.0);
        if ask {
            let targets = targets.to_vec();
            app.ask_unsubscribe(&message_id, m.from.display(), &targets);
        }
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
                if blocked > 0 && !allowed && !in_junk {
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
                if in_junk {
                    ui.add_enabled(false, chip)
                        .on_disabled_hover_text("Uitgeschakeld in Ongewenste e-mail. Klik op Geen ongewenst als je deze afzender vertrouwt.");
                } else if ui.add(chip).on_hover_text("Opslaan in je map Downloads. Mailcraft opent bijlagen niet zelf.").clicked() {
                    save = Some(a.clone());
                }
            }
        });
        if let Some(a) = save {
            app.save_attachment(&a);
        }
        // Thumbnails of the pictures, as Outlook previews them; a click opens the full picture.
        // Not in Junk, where nothing of a message is decoded beyond its text.
        let mut open = None;
        if !in_junk && b.attachments.iter().any(attachments::previewable) {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (i, a) in b.attachments.iter().enumerate().filter(|(_, a)| attachments::previewable(a)) {
                    let uri = attachments::preview_uri(&message_id, i);
                    let image = egui::Image::from_bytes(uri.clone(), egui::load::Bytes::Shared(a.bytes.clone()))
                        .max_size(vec2(200.0, 120.0))
                        .fit_to_original_size(1.0)
                        .corner_radius(CornerRadius::same(3));
                    let label = attachments::preview_label(&a.name);
                    if ui
                        .add(egui::Button::image_and_text(image, RichText::new(&a.name).size(11.0).color(t.text_dim)).fill(t.panel))
                        .on_hover_text(&label)
                        .clicked()
                    {
                        open = Some(attachments::Preview { attachment: a.clone(), uri });
                    }
                }
            });
        }
        if open.is_some() {
            app.preview = open;
        }
        ui.add_space(8.0);
    }
    ui.add_space(6.0);
    if let Some(b) = body.as_ref()
        && let Some(doc) = b.rich.as_ref()
    {
        let remote = app.remote.snapshot(&message_id);
        let clicked = egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| html::show(ui, t, doc, &b.images, &message_id, remote.as_ref(), !in_junk))
            .inner;
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

/// The first line of the strip above a message that shows signs of phishing.
pub(crate) const PHISHING_TITLE: &str = "⚠ Pas op: dit bericht heeft kenmerken van phishing.";
/// The strip above a message of an ignored conversation.
pub(crate) const IGNORED_NOTICE: &str = "🚫 Dit gesprek wordt genegeerd: nieuwe berichten erin gaan meteen naar Verwijderde items.";
/// The strip above a message in the Junk folder.
pub(crate) const JUNK_NOTICE: &str = "⚠ Dit bericht staat in Ongewenste e-mail. Links, bijlagen en afbeeldingen van internet zijn uitgeschakeld tot je het als Geen ongewenst aanmerkt.";

/// What points to phishing in `m`, from its sender (against the contacts and the person's own
/// addresses) and, once fetched, its body: the server's authentication results, the links and
/// the attachments.
fn message_warnings(app: &MailApp, m: &mailcraft_model::MessageHeader, body: Option<&mailcraft_engine::Body>) -> Vec<Warning> {
    let s = &app.engine.session;
    let own: Vec<&str> = s.accounts.iter().map(|a| a.address.email()).collect();
    let mut out = mailcraft_model::sender_warnings(&m.from, &s.contacts, &own);
    if let Some(b) = body {
        out.extend(mailcraft_model::authentication_warning(&b.authentication));
        out.extend(mailcraft_model::link_warnings(&b.rich.as_ref().map(|d| d.links()).unwrap_or_default()));
        out.extend(mailcraft_model::attachment_warnings(b.attachments.iter().map(|a| a.name.as_str())));
    }
    out
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
        h.state_mut().compose = None;
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::R);
        h.run();
        let e = h.state().calendar.editor.as_ref().expect("reply with meeting opens the appointment window");
        assert!(!e.attendees.is_empty(), "the sender is invited: {:?}", e.attendees);
        h.state_mut().calendar.editor = None;
        h.run();
        h.state_mut().compose = Some(Compose::new(ComposeKind::New, None));
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
        h.get_by_label("Werkweek").click();
        h.run();
        assert_eq!(h.state().calendar.view, calendar::View::WorkWeek);
        if chrono::Datelike::weekday(&chrono::Local::now().date_naive()).num_days_from_monday() < 5 {
            assert!(h.query_all_by_label_contains("Overleg ontwerp").count() >= 1, "today's appointment shows in the work week on a weekday");
        }
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
    fn an_appointment_with_attendees_offers_verzenden_and_needs_a_real_account() {
        let mut h = harness();
        let mut e = calendar::Editor::new_at(chrono::Local::now().naive_local());
        e.title = "Overleg met Sanne".into();
        e.attendees = "Sanne <sanne@example.org>".into();
        h.state_mut().calendar.editor = Some(e);
        h.run();
        assert!(h.query_by_label("Opslaan").is_none(), "with attendees the button sends");
        h.get_by_label("Verzenden").click();
        h.run();
        h.get_by_label(meeting::NO_ACCOUNT);
        assert!(h.state().calendar.editor.is_some(), "the window stays open");
        assert!(h.state().engine.session.appointments.iter().all(|a| a.title != "Overleg met Sanne"), "nothing was saved");
        h.state_mut().calendar.editor.as_mut().expect("editor").attendees = "sanne@".into();
        h.run();
        h.get_by_label("Verzenden").click();
        h.run();
        h.get_by_label(meeting::BAD_ADDRESSES);
    }

    #[test]
    fn a_contact_group_is_made_mailed_and_offered_while_an_address_is_typed() {
        let mut h = harness();
        h.state_mut().module = Module::People;
        h.run();
        h.get_by_label(groups::NEW).click();
        h.run();
        {
            let e = h.state_mut().contacts.group_editor.as_mut().expect("the group window");
            e.name = "Team ontwerp".into();
            e.members = "Sanne <sanne@example.org>\nravi@example.org".into();
        }
        h.get_by_label("Opslaan").click();
        h.run();
        let groups = h.state().engine.session.groups.clone();
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].name.as_str(), groups[0].members.len()), ("Team ontwerp", 2));
        assert!(h.state().contacts.group_editor.is_none());
        assert_eq!(h.state().contacts.group.as_deref(), Some(groups[0].id.as_str()), "the new group is shown");
        h.get_by_label("👥 Team ontwerp (2)");
        h.get_by_label(groups::MAIL).click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| c.to.as_str()), Some("Sanne <sanne@example.org>; ravi@example.org; "));
        // Typing the group's name in an address field offers its members.
        h.state_mut().module = Module::Mail;
        h.state_mut().compose.as_mut().expect("the compose window").to = "tea".into();
        h.run();
        h.get_by_label("👥 Team ontwerp (2)").click();
        h.run();
        assert_eq!(h.state().compose.as_ref().map(|c| c.to.as_str()), Some("Sanne <sanne@example.org>; ravi@example.org; "));
    }

    #[test]
    fn a_sender_is_opened_or_added_as_a_contact_from_the_reading_pane() {
        let mut h = harness();
        h.state_mut().engine.session.selected_message = Some("m1".into());
        h.run();
        h.get_by_label("Contact openen").click();
        h.run();
        assert!(h.state().module == Module::People);
        assert_eq!(h.state().engine.session.selected_contact().map(|c| c.name.as_str()), Some("Sanne de Vries"), "the known sender's card opens");
        assert!(h.state().contacts.editor.is_none());

        h.state_mut().module = Module::Mail;
        h.state_mut().engine.session.selected_message = Some("m3".into());
        h.run();
        h.get_by_label("Toevoegen aan contacten").click();
        h.run();
        assert!(h.state().module == Module::People);
        let e = h.state().contacts.editor.as_ref().expect("the form for a new contact");
        assert_eq!((e.id.as_deref(), e.name.as_str(), e.emails.as_str()), (None, "Nieuwsbrief", "news@example.org"));
        h.get_by_label("Opslaan").click();
        h.run();
        assert_eq!(h.state().engine.session.contact_with_email("news@example.org").map(|c| c.name.as_str()), Some("Nieuwsbrief"));
    }

    #[test]
    fn an_urgent_message_gets_the_strip_over_it_and_a_normal_one_does_not() {
        let mut h = harness();
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        h.get_by_label(importance::HIGH_STRIP);
        h.get_by_label("Re: Offerte drukwerk").click();
        h.run();
        assert!(h.query_by_label(importance::HIGH_STRIP).is_none() && h.query_by_label(importance::LOW_STRIP).is_none());
    }

    #[test]
    fn a_message_with_signs_of_phishing_gets_a_warning_strip_that_can_block_the_sender() {
        use mailcraft_model::{Address, Flags, MessageHeader};
        let mut h = harness();
        let from = Address::new(Some("Sanne de Vries"), "sanne@examp1e.org").unwrap();
        let m = MessageHeader {
            id: "inbox#phish".into(),
            folder_id: "inbox".into(),
            remote_id: String::new(),
            from,
            to: Vec::new(),
            subject: "Uw account".into(),
            date: 0,
            preview: String::new(),
            flags: Flags::default(),
            has_attachments: true,
            message_id: None,
            categories: Vec::new(),
            importance: Default::default(),
        };
        let body = mailcraft_engine::Body {
            rich: Some(mailcraft_html::parse(r#"<p><a href="https://evil.example/login">https://www.bank.nl/inloggen</a></p>"#)),
            attachments: vec![mailcraft_mime::Attachment {
                name: "factuur.pdf.exe".into(),
                mime: "application/octet-stream".into(),
                size: 3,
                bytes: std::sync::Arc::from(&b"abc"[..]),
            }],
            authentication: vec!["mx.example; spf=pass; dmarc=fail header.from=examp1e.org".into()],
            ..Default::default()
        };
        h.state_mut().engine.session.messages.push(m);
        h.state_mut().engine.session.bodies.insert("inbox#phish".into(), body);
        h.state_mut().engine.session.selected_message = Some("inbox#phish".into());
        h.run();
        h.get_by_label(PHISHING_TITLE);
        let expected = [
            Warning::Impersonation { name: "Sanne de Vries".into() },
            Warning::LookalikeDomain { domain: "examp1e.org".into(), like: "example.org".into() },
            Warning::Unauthenticated { checks: "DMARC".into() },
            Warning::MisleadingLink { shown: "www.bank.nl".into(), host: "evil.example".into() },
            Warning::RiskyAttachment { name: "factuur.pdf.exe".into() },
        ];
        for w in &expected {
            h.get_by_label(format!("• {}", w.text()).as_str());
        }
        h.get_by_label("Afzender blokkeren").click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.message("inbox#phish").map(|m| m.folder_id.as_str()), Some("junk"));
        assert_eq!(s.junk.blocked, ["sanne@examp1e.org"]);

        // An ordinary message has no strip.
        h.state_mut().engine.session.selected_message = Some("m1".into());
        h.run();
        assert!(h.query_by_label(PHISHING_TITLE).is_none());
    }

    #[test]
    fn a_picture_attachment_shows_a_thumbnail_that_opens_the_preview_window() {
        let mut h = harness();
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf,
            0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xc9, 0xfe, 0x92, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        let picture =
            mailcraft_mime::Attachment { name: "foto.png".into(), mime: "image/png".into(), size: png.len(), bytes: std::sync::Arc::from(png) };
        let paper =
            mailcraft_mime::Attachment { name: "bon.pdf".into(), mime: "application/pdf".into(), size: 3, bytes: std::sync::Arc::from(&b"abc"[..]) };
        assert!(attachments::previewable(&picture) && !attachments::previewable(&paper));
        let body = mailcraft_engine::Body { text: "Zie bijlage".into(), attachments: vec![paper, picture], ..Default::default() };
        h.state_mut().engine.session.bodies.insert("m3".into(), body);
        h.state_mut().engine.session.selected_message = Some("m3".into());
        h.run();
        h.get_by_label_contains("📎 bon.pdf");
        assert!(h.query_by_label("Voorbeeld van bon.pdf").is_none(), "a PDF is only saved, never decoded");
        h.get_by_label("foto.png").click();
        h.run();
        assert_eq!(h.state().preview.as_ref().map(|p| p.attachment.name.as_str()), Some("foto.png"));
        h.get_by_label("Voorbeeld van foto.png");
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().preview.is_none());
    }

    #[test]
    fn links_attachments_and_pictures_are_switched_off_in_the_junk_folder() {
        use egui_kittest::kittest::NodeT;
        let mut h = harness();
        let body = mailcraft_engine::Body {
            rich: Some(mailcraft_html::parse(
                r#"<p><a href="https://example.nl/actie">Klik hier</a> <img src="https://t.example/p.gif" alt="pixel"></p>"#,
            )),
            attachments: vec![mailcraft_mime::Attachment {
                name: "bon.pdf".into(),
                mime: "application/pdf".into(),
                size: 3,
                bytes: std::sync::Arc::from(&b"abc"[..]),
            }],
            ..Default::default()
        };
        h.state_mut().engine.session.bodies.insert("m3".into(), body);
        h.state_mut().engine.session.run("junk.block", &json!({ "id": "m3" })).unwrap();
        h.state_mut().engine.session.selected_message = Some("m3".into());
        h.run();
        h.get_by_label(JUNK_NOTICE);
        assert!(h.get_by_label_contains("bon.pdf").accesskit_node().is_disabled(), "the attachment cannot be saved from here");
        assert!(h.query_by_label("Afbeeldingen laden").is_none(), "pictures on the internet cannot be asked for");
        h.get_by_label("Klik hier").hover();
        h.run();
        assert!(h.query_by_label_contains(html::LINKS_OFF).is_some(), "the link says why it does nothing");

        // "Geen ongewenst" brings the message back, and everything works again.
        h.get_by_label("Geen ongewenst").click();
        h.run();
        assert_eq!(h.state().engine.session.message("m3").map(|m| m.folder_id.as_str()), Some("inbox"));
        // The move dropped the selection, as any move does; back on the message everything works.
        h.state_mut().engine.session.selected_message = Some("m3".into());
        h.run();
        assert!(h.query_by_label(JUNK_NOTICE).is_none());
        assert!(!h.get_by_label_contains("bon.pdf").accesskit_node().is_disabled());
        assert!(h.query_by_label("Afbeeldingen laden").is_some());
    }

    #[test]
    fn the_list_is_headed_by_date_groups_until_the_view_turns_them_off() {
        let mut h = harness();
        let mut today = h.state().engine.session.message("m1").expect("sample mail").clone();
        today.id = "inbox#today".into();
        today.date = chrono::Local::now().timestamp();
        h.state_mut().engine.session.messages.push(today);
        h.run();
        h.get_by_label("Vandaag");
        assert!(h.query_by_label("Later").is_none());
        let headings = ["Gisteren", "Deze week", "Vorige week", "Deze maand", "Vorige maand", "Ouder"];
        assert_eq!(headings.iter().filter(|g| h.query_by_label(g).is_some()).count(), 1, "the sample mail of one afternoon makes one more group");

        h.get_by_label("Beeld").click();
        h.run();
        h.get_by_label("Groepen").click();
        h.run();
        assert!(!h.state().view.groups);
        h.get_by_label("Start").click();
        h.run();
        assert!(h.query_by_label("Vandaag").is_none());
    }

    #[test]
    fn a_flagged_message_whose_day_has_come_shows_in_the_reminders_window() {
        let mut h = harness();
        h.state_mut().run("message.follow_up", json!({ "when": "today", "id": "m3" }));
        h.run();
        h.get_by_label("⚑ Opvolgen: Wat er deze maand speelt");
        h.get_by_label("Vandaag");
        assert!(h.state().announced_reminders.iter().any(|(k, _)| k == "followup:m3"), "the window asked for attention once");
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.query_by_label("⚑ Opvolgen: Wat er deze maand speelt").is_none(), "closed; the flag stays");
        assert!(h.state().engine.session.message("m3").unwrap().flags.flagged);
        assert!(h.state().engine.session.follow_ups.iter().any(|f| f.dismissed));
    }

    #[test]
    fn an_invitation_in_the_mail_opens_the_appointment_window_filled_in() {
        let mut h = harness();
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:k1@x\r\nDTSTART:20261021T080000Z\r\nDTEND:20261021T093000Z\r\nSUMMARY:Kickoff\r\nLOCATION:Kantoor\r\nORGANIZER;CN=Sanne:mailto:sanne@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let body = mailcraft_engine::Body {
            text: "Kom je ook?".into(),
            attachments: vec![mailcraft_mime::Attachment {
                name: "invite.ics".into(),
                mime: "text/calendar".into(),
                size: ics.len(),
                bytes: std::sync::Arc::from(ics.as_bytes()),
            }],
            ..Default::default()
        };
        h.state_mut().engine.session.bodies.insert("m3".into(), body);
        h.state_mut().engine.session.selected_message = Some("m3".into());
        h.run();
        h.get_by_label("📅 Uitnodiging: Kickoff");
        h.get_by_label_contains("Kantoor · van Sanne");
        h.get_by_label("Aan agenda toevoegen").click();
        h.run();
        {
            let e = h.state().calendar.editor.as_ref().expect("the appointment window");
            assert_eq!((e.title.as_str(), e.location.as_str(), &e.id, &e.calendar_id), ("Kickoff", "Kantoor", &None, &None));
            assert_eq!(e.start_date, "21-10-2026");
        }
        h.get_by_label("Opslaan").click();
        h.run();
        assert!(h.state().engine.session.appointments.iter().any(|a| a.title == "Kickoff" && a.calendar_id == "local"));
        h.get_by_label("Staat al in je agenda.");
        assert!(h.query_by_label("Aan agenda toevoegen").is_none());
    }

    #[test]
    fn a_newsletter_offers_afmelden_and_asks_before_the_mail_goes_out() {
        let mut h = harness();
        {
            let s = &mut h.state_mut().engine.session;
            let body = mailcraft_engine::Body {
                text: "Nieuws".into(),
                unsubscribe: vec!["https://example.org/u".into(), "mailto:leave@example.org?subject=stop".into()],
                ..Default::default()
            };
            s.bodies.insert("m3".into(), body);
            s.selected_message = Some("m3".into());
        }
        h.run();
        h.get_by_label(unsubscribe::STRIP);
        h.get_by_label("Afmelden…").click();
        h.run();
        let c = h.state().unsubscribe.clone().expect("the question is open");
        assert!(matches!(c.way, unsubscribe::Way::Mail { ref subject, .. } if subject == "stop"), "mail comes before the web page");
        h.get_by_label("Annuleren").click();
        h.run();
        assert!(h.state().unsubscribe.is_none() && h.state().engine.session.outbox.is_empty());
        h.get_by_label("Afmelden…").click();
        h.run();
        h.get_by_label("Ja, afmelden").click();
        h.run();
        assert!(h.state().unsubscribe.is_none());
        assert_eq!(h.state().engine.session.outbox.len(), 1, "the request was handed to the outbox");
        assert!(h.state().notice.is_some(), "the sample account cannot send, and the person hears so");
    }

    #[test]
    fn flagged_mail_shows_in_the_tasks_module_opens_in_mail_and_is_done_from_its_checkbox() {
        let mut h = harness();
        // Tomorrow, not today: a due follow-up would open the reminders window over the list.
        h.state_mut().run("message.follow_up", json!({ "when": "tomorrow", "id": "m1" }));
        h.state_mut().run("message.flag", json!({ "id": "m3" }));
        h.state_mut().module = Module::Tasks;
        h.run();
        h.get_by_label(tasks::FLAGGED_HEADING);
        assert!(h.query_by_label("Planning voor donderdag").is_some() && h.query_by_label("Wat er deze maand speelt").is_some());
        h.get_by_label("Voltooid").click();
        h.run();
        assert!(h.query_by_label(tasks::FLAGGED_HEADING).is_none(), "the done filter is about tasks; flagged mail is open by nature");
        h.get_by_label("Open").click();
        h.run();
        h.get_by_label("Vlag weghalen van Wat er deze maand speelt").click();
        h.run();
        assert!(!h.state().engine.session.message("m3").unwrap().flags.flagged);
        assert!(h.query_by_label("Wat er deze maand speelt").is_none());
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        assert_eq!(h.state().module, Module::Mail);
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some("m1"));
    }

    #[test]
    fn a_message_is_flagged_for_tomorrow_from_the_menu_and_marked_done_from_the_strip() {
        let mut h = harness();
        h.run();
        h.get_by_label("Planning voor donderdag").click_secondary();
        h.run();
        h.get_by_label_contains("Opvolgen").click();
        h.run();
        h.get_by_label("Morgen").click();
        h.run();
        let s = &h.state().engine.session;
        assert!(s.message("m1").unwrap().flags.flagged);
        let tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap();
        assert_eq!(s.follow_up("m1").and_then(mailcraft_engine::followup::due_day), Some(tomorrow));
        h.get_by_label(format!("⚑ Opvolgen vóór {}", dates::day_name(tomorrow)).as_str());
        h.get_by_label("Voltooid").click();
        h.run();
        let s = &h.state().engine.session;
        assert!(!s.message("m1").unwrap().flags.flagged && s.follow_up("m1").is_none());
    }

    #[test]
    fn several_messages_are_selected_with_ctrl_shift_and_ctrl_a_and_deleted_together() {
        use egui::{Key, Modifiers};
        let mut h = harness();
        h.run();
        let ids: Vec<String> = h.state().engine.session.visible_messages().iter().map(|m| m.id.clone()).collect();
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some("m1"));
        h.get_by_label("Wat er deze maand speelt").click_modifiers(Modifiers::COMMAND);
        h.run();
        assert_eq!(h.state().engine.session.marked, ["m3"]);
        h.get_by_label("2 berichten geselecteerd");
        h.get_by_label(MULTI_HINT);

        // Shift-click selects everything between the selected message and the clicked one.
        let (a, c) = (ids.iter().position(|i| i == "m1").unwrap(), ids.iter().position(|i| i == "m3").unwrap());
        h.get_by_label("Wat er deze maand speelt").click_modifiers(Modifiers::SHIFT);
        h.run();
        assert_eq!(h.state().engine.session.selection().len(), a.abs_diff(c) + 1);
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some("m1"), "the selected message keeps the reading pane");

        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run();
        assert_eq!(h.state().engine.session.selection().len(), ids.len());
        h.key_press(Key::Delete);
        h.run();
        let s = &h.state().engine.session;
        assert!(ids.iter().all(|id| s.message(id).unwrap().folder_id == "deleted"), "Delete takes every selected message");
        assert!(s.selected_message.is_none() && s.marked.is_empty());

        // A plain click opens one message again.
        h.get_by_label("Verwijderde items").click();
        h.run();
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        assert_eq!(h.state().engine.session.selection(), ["m1"]);
    }

    #[test]
    fn a_search_is_saved_as_a_search_folder_and_run_again_from_the_folder_pane() {
        let mut h = harness();
        h.run();
        assert!(h.query_by_label(search_folders::HEADING).is_none());
        {
            let s = &mut h.state_mut().engine.session;
            s.search = "offerte".into();
            s.search_all = true;
        }
        h.run();
        h.get_by_label("Zoekmap").click();
        h.run();
        assert_eq!(h.state().search_folder_form.as_ref().map(|f| f.name.as_str()), Some("offerte"), "the words are the first name");
        h.state_mut().search_folder_form.as_mut().expect("form").name = "Offertes".into();
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        assert!(h.state().search_folder_form.is_none());
        let saved = h.state().view.search_folders.clone();
        assert_eq!(saved.len(), 1);
        assert_eq!((saved[0].name.as_str(), saved[0].query.as_str(), saved[0].all_folders), ("Offertes", "offerte", true));
        h.get_by_label(search_folders::HEADING);

        h.state_mut().engine.session.search.clear();
        h.state_mut().engine.session.search_all = false;
        h.run();
        h.get_by_label("Offertes").click();
        h.run();
        let s = &h.state().engine.session;
        assert!(s.search == "offerte" && s.search_all, "the saved search runs again");
        assert!(s.visible_messages().iter().all(|m| m.subject.to_lowercase().contains("offerte")));

        h.get_by_label("Offertes").click_secondary();
        h.run();
        h.get_by_label("Zoekmap verwijderen").click();
        h.run();
        assert!(h.state().view.search_folders.is_empty() && h.query_by_label(search_folders::HEADING).is_none());
    }

    #[test]
    fn a_folder_is_pinned_under_favorites_from_its_menu_and_taken_off_from_the_ribbon() {
        let mut h = harness();
        h.run();
        assert!(h.query_by_label("★ Favorieten").is_none());
        h.get_by_label("Archief").click_secondary();
        h.run();
        h.get_by_label("Toevoegen aan Favorieten").click();
        h.run();
        assert_eq!(h.state().view.favorites, ["archive"]);
        h.get_by_label("★ Favorieten");
        let rows: Vec<_> = h.get_all_by_label("Archief").collect();
        assert_eq!(rows.len(), 2, "the folder shows under Favorieten and under its account");
        rows[0].click();
        h.run();
        assert_eq!(h.state().engine.session.selected_folder.as_deref(), Some("archive"));

        h.get_by_label("Map").click();
        h.run();
        h.get_by_label("Favoriet").click();
        h.run();
        assert!(h.state().view.favorites.is_empty());
        assert!(h.query_by_label("★ Favorieten").is_none());
    }

    #[test]
    fn a_conversation_is_cleaned_up_from_the_menu_after_the_question_says_how_many_go() {
        let mut h = harness();
        use mailcraft_model::{Flags, MessageHeader};
        {
            let s = &mut h.state_mut().engine.session;
            let newest = s.message("m2").expect("sample mail").clone();
            let older = |id: &str, subject: &str, hours: i64| MessageHeader {
                id: id.into(),
                subject: subject.into(),
                date: newest.date - hours * 3600,
                flags: Flags { seen: true, ..Flags::default() },
                has_attachments: false,
                categories: Vec::new(),
                message_id: Some(format!("<{id}@example.org>")),
                ..newest.clone()
            };
            s.messages.push(older("m8", "Offerte drukwerk", 30));
            s.messages.push(older("m9", "Re: offerte drukwerk", 20));
            s.messages
                .push(MessageHeader { flags: Flags { seen: true, flagged: true, answered: false }, ..older("m10", "RE: Offerte drukwerk", 10) });
        }
        h.run();
        h.get_by_label("Re: Offerte drukwerk").click_secondary();
        h.run();
        h.get_by_label("Gesprek opruimen…").click();
        h.run();
        assert_eq!(h.state().cleanup.as_ref().map(|c| c.count), Some(2), "the newest and the flagged one stay");
        h.get_by_label_contains("2 oudere berichten uit het gesprek");
        h.get_by_label("Annuleren").click();
        h.run();
        assert!(h.state().cleanup.is_none() && h.state().engine.session.message("m8").unwrap().folder_id == "inbox");
        h.state_mut().ask_cleanup_conversation("m2");
        h.run();
        h.get_by_label("Ja, opruimen").click();
        h.run();
        let s = &h.state().engine.session;
        assert!(h.state().cleanup.is_none());
        assert_eq!((s.message("m8").unwrap().folder_id.as_str(), s.message("m9").unwrap().folder_id.as_str()), ("deleted", "deleted"));
        assert_eq!((s.message("m10").unwrap().folder_id.as_str(), s.message("m2").unwrap().folder_id.as_str()), ("inbox", "inbox"));
    }

    #[test]
    fn a_conversation_is_ignored_from_the_menu_and_taken_back_from_the_strip() {
        let mut h = harness();
        h.run();
        h.get_by_label("Wat er deze maand speelt").click_secondary();
        h.run();
        h.get_by_label("Gesprek negeren").click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.message("m3").map(|m| m.folder_id.as_str()), Some("deleted"));
        assert_eq!(s.ignored.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["wat er deze maand speelt"]);

        h.state_mut().engine.session.selected_message = Some("m3".into());
        h.run();
        h.get_by_label(IGNORED_NOTICE);
        h.get_by_label("Niet meer negeren").click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.message("m3").map(|m| m.folder_id.as_str()), Some("inbox"));
        assert!(s.ignored.is_empty());
    }

    #[test]
    fn a_sender_is_blocked_from_the_menu_and_trusted_back_from_the_junk_folder() {
        let mut h = harness();
        h.run();
        h.get_by_label("Wat er deze maand speelt").click_secondary();
        h.run();
        h.get_by_label("Afzender blokkeren").click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.message("m3").map(|m| m.folder_id.as_str()), Some("junk"));
        assert_eq!(s.junk.blocked, ["news@example.org"]);
        assert!(h.state().notice.is_none());

        // The window lists the sender and lets it go again.
        h.state_mut().junk = Some(junk::Options::default());
        h.run();
        h.get_by_label("Weghalen").click();
        h.run();
        assert!(h.state().engine.session.junk.blocked.is_empty());
        if let Some(o) = h.state_mut().junk.as_mut() {
            o.entry = "spam.example".into();
        }
        h.run();
        h.get_by_label("Blokkeren").click();
        h.run();
        assert_eq!(h.state().engine.session.junk.blocked, ["spam.example"]);
        assert_eq!(h.state().junk.as_ref().map(|o| o.entry.as_str()), Some(""), "the field is cleared");
        h.get_by_label("Hoog").click();
        h.run();
        assert_eq!(h.state().engine.session.junk.level, mailcraft_model::JunkLevel::High);
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().junk.is_none());

        // Reading the message in the Junk folder: the strip's button brings it back and trusts the sender.
        h.state_mut().run("folder.select", json!({ "id": "junk" }));
        h.state_mut().run("message.select", json!({ "id": "m3" }));
        h.run();
        h.get_by_label("Geen ongewenst").click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.message("m3").map(|m| m.folder_id.as_str()), Some("inbox"));
        assert_eq!(s.junk.safe, ["news@example.org"]);
    }

    #[test]
    fn the_right_click_menu_on_a_message_makes_a_task_of_it() {
        let mut h = harness();
        h.run();
        h.get_by_label("Planning voor donderdag").click_secondary();
        h.run();
        assert!(h.query_by_label("Afspraak maken van dit bericht").is_some(), "the menu is open");
        h.get_by_label("Taak maken van dit bericht").click();
        h.run();
        assert!(h.state().module == Module::Tasks, "the tasks module shows the new form");
        assert_eq!(h.state().tasks.editor.as_ref().map(|e| e.title.as_str()), Some("Planning voor donderdag"));
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some("m1"));
        h.get_by_label("Opslaan").click();
        h.run();
        assert!(h.state().engine.session.tasks.iter().any(|t| t.title == "Planning voor donderdag" && t.notes.starts_with("Van: Sanne")));
    }

    #[test]
    fn a_new_item_starts_from_every_module_with_outlooks_keys() {
        use egui::{Key, Modifiers};
        let keys = Modifiers::COMMAND | Modifiers::SHIFT;
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        h.key_press_modifiers(keys, Key::M);
        h.run();
        assert!(h.state().compose.as_ref().is_some_and(|c| c.subject.is_empty()), "Ctrl+Shift+M writes a new message from the calendar");
        h.state_mut().compose = None;
        h.key_press_modifiers(keys, Key::K);
        h.run();
        assert!(
            h.state().module == Module::Tasks && h.state().tasks.editor.as_ref().is_some_and(|e| e.id.is_none()),
            "Ctrl+Shift+K: a new task in its module"
        );
        h.state_mut().tasks.editor = None;
        h.key_press_modifiers(keys, Key::C);
        h.run();
        assert!(
            h.state().module == Module::People && h.state().contacts.editor.as_ref().is_some_and(|e| e.id.is_none()),
            "Ctrl+Shift+C: a new contact in its module"
        );
        h.state_mut().contacts.editor = None;
        h.state_mut().module = Module::Mail;
        h.run();
        h.key_press_modifiers(keys, Key::A);
        h.run();
        assert!(h.state().calendar.editor.as_ref().is_some_and(|e| e.id.is_none()), "Ctrl+Shift+A opens a new appointment from the mail module");
        assert!(h.state().module == Module::Mail, "the appointment window needs no module switch");
    }

    #[test]
    fn the_conversation_view_folds_a_subject_behind_its_newest_message() {
        let mut h = harness();
        let mut earlier = h.state().engine.session.message("m2").expect("Re: Offerte drukwerk").clone();
        earlier.id = "m9".into();
        earlier.subject = "Offerte drukwerk".into();
        earlier.date -= 86_400;
        h.state_mut().engine.session.messages.push(earlier);
        h.run();
        assert!(h.query_by_label("Offerte drukwerk").is_some(), "every message on its own while the view is off");
        assert!(h.query_by_label("Gesprek uitvouwen").is_none());

        h.get_by_label("Beeld").click();
        h.run();
        h.get_by_label("Gesprekken").click();
        h.run();
        assert!(h.state().view.conversations);
        h.get_by_label("Start").click();
        h.run();
        assert!(h.query_by_label("Re: Offerte drukwerk").is_some(), "the newest message heads the conversation");
        assert!(h.query_by_label("Offerte drukwerk").is_none(), "the older one is folded away");
        assert_eq!(h.query_all_by_label("Gesprek uitvouwen").count(), 1, "only the conversation of two has a fold control");
        h.get_by_label("Gesprek uitvouwen").click();
        h.run();
        h.get_by_label("Offerte drukwerk").click();
        h.run();
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some("m9"));
        h.get_by_label("Gesprek inklappen").click();
        h.run();
        assert_eq!(
            h.query_all_by_label("Offerte drukwerk").count(),
            2,
            "the row stays (next to the reading pane's title): the conversation holds the selected message"
        );
        h.state_mut().engine.session.selected_message = Some("m0".into());
        h.run();
        assert!(h.query_by_label("Offerte drukwerk").is_none(), "folded again once the selection has left it");
    }

    #[test]
    fn the_calendar_and_tasks_search_boxes_narrow_what_is_shown() {
        use egui::{Key, Modifiers};
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::E);
        h.run();
        assert_eq!(h.ctx.memory(|m| m.focused()), Some(egui::Id::new(calendar::SEARCH_BOX)), "Ctrl+E goes to the calendar's search box");
        h.ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(calendar::SEARCH_BOX)));
        h.state_mut().calendar.query = "sanne".into();
        h.run();
        assert!(h.query_by_label("Lunch met Sanne").is_some(), "the results list names the match");
        assert!(h.query_by_label_contains("Overleg ontwerp").is_none(), "the month view is gone while searching");
        assert!(h.query_by_label("1 afspraak gevonden.").is_some());
        h.get_by_label("Lunch met Sanne").click();
        h.run();
        assert_eq!(h.state().calendar.editor.as_ref().map(|e| e.title.as_str()), Some("Lunch met Sanne"), "a click opens the appointment");
        h.state_mut().calendar.editor = None;
        h.state_mut().calendar.query = "weekstart".into();
        h.run();
        assert!(h.query_all_by_label_contains("Weekstart").count() > 50, "a weekly series gives one row per occurrence in three years");
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().calendar.query.is_empty(), "Esc clears the search");
        assert!(h.query_by_label_contains("Overleg ontwerp").is_some(), "and the month view is back");

        h.state_mut().module = Module::Tasks;
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::E);
        h.run();
        assert_eq!(h.ctx.memory(|m| m.focused()), Some(egui::Id::new(tasks::SEARCH_BOX)));
        h.ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(tasks::SEARCH_BOX)));
        h.state_mut().tasks.query = "portfolio".into();
        h.run();
        assert!(h.query_by_label("Portfolio bijwerken").is_some());
        assert!(h.query_by_label("Offerte drukwerk beantwoorden").is_none(), "the other open task is filtered out");
        h.state_mut().tasks.query = "niets-hiervan".into();
        h.run();
        assert!(h.query_by_label("Geen taak gevonden.").is_some());
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().tasks.query.is_empty());
        assert!(h.query_by_label("Offerte drukwerk beantwoorden").is_some());
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
    fn a_task_dragged_onto_the_calendar_opens_an_appointment_for_it() {
        use chrono::Datelike;
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.view = calendar::View::Day;
        h.state_mut().calendar.focus = calendar::today();
        h.run();
        let from = h.get_by_label("Portfolio bijwerken").rect().center();
        let today = calendar::today();
        let column =
            h.get_by_label(format!("{} {} {}", calendar::weekday_name(today), today.day(), calendar::month_name(today.month())).as_str()).rect();
        // 09:10 in the hour grid: nine hours of 44 px below the column's midnight.
        let to = egui::pos2(column.center().x, column.top() + 9.0 * 44.0 + 7.0);
        h.drag_at(from);
        h.run();
        for step in 1..=4 {
            h.hover_at(from.lerp(to, step as f32 / 4.0));
            h.run();
        }
        // Let go over the column; the pointer stays where it is, as a mouse does.
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run();
        let e = h.state().calendar.editor.as_ref().expect("the appointment window opens");
        assert_eq!((e.id.as_deref(), e.title.as_str(), e.all_day), (None, "Portfolio bijwerken", false));
        assert_eq!((e.start_time.as_str(), e.end_time.as_str()), ("09:00", "10:00"));
        assert_eq!(e.start_date, today.format("%d-%m-%Y").to_string());
    }

    #[test]
    fn dragging_the_bottom_edge_of_an_appointment_changes_its_end() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.view = calendar::View::Day;
        h.state_mut().calendar.focus = calendar::today();
        h.run();
        let before = h.state().engine.session.appointment("sample-0").expect("today's sample appointment").clone();
        let handle = h.get_by_label("Eindtijd van Overleg ontwerp verslepen").rect();
        let from = handle.center();
        h.drag_at(from);
        h.run();
        // One hour further down the grid, in small steps as a pointer moves.
        for step in 1..=4 {
            h.hover_at(from + vec2(0.0, 11.0 * step as f32));
            h.run();
        }
        assert_eq!(h.state().calendar.resizing.as_ref().map(|r| r.end), Some(before.end + 3_600), "the block follows the pointer");
        h.drop_at(from + vec2(0.0, 44.0));
        h.run();
        let after = h.state().engine.session.appointment("sample-0").unwrap();
        assert_eq!((after.start, after.end), (before.start, before.end + 3_600));
        assert!(h.state().calendar.resizing.is_none());
        assert!(h.state().notice.is_none());
    }

    #[test]
    fn the_window_title_counts_the_unread_inbox_mail_and_follows_it() {
        assert_eq!(window_title(0), "Mailcraft");
        assert_eq!(window_title(3), "Mailcraft (3)");
        let mut h = harness();
        h.run();
        let unread = h.state().engine.session.inbox_unread();
        assert!(unread > 0);
        assert_eq!(h.state().title.as_deref(), Some(format!("Mailcraft ({unread})").as_str()));
        h.get_by_label("Map").click();
        h.run();
        h.get_by_label("Alles gelezen").click();
        h.run();
        assert_eq!(h.state().title.as_deref(), Some("Mailcraft"));
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
        h.get_by_label("Uit").click();
        h.run();
        assert_eq!(h.state().view.reading_pane, view::ReadingPane::Off);
        assert!(h.query_by_label("Selecteer een bericht om het te lezen.").is_none(), "no reading pane");
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        assert!(!h.state().message_window, "a single click only selects");
        // The keys of the mail module work from the Start tab.
        h.get_by_label("Start").click();
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(h.state().message_window);
        h.get_by_label("✉ Planning voor donderdag");
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(!h.state().message_window && h.query_by_label("✉ Planning voor donderdag").is_none());
        h.get_by_label("Beeld").click();
        h.run();
        h.get_by_label("Rechts").click();
        h.run();
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
