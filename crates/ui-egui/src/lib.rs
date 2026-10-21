//! Merel's front end: an Outlook-style window drawn with egui in the ArtCraft look.
//!
//! Layout, top to bottom: the tab strip and ribbon, then the module rail (Mail, Calendar,
//! People, Tasks), the folder pane, the message list and the reading pane, then the status bar.
//! Every button runs a command id through [`merel_engine::Session::run`]; nothing here
//! changes the session directly.

mod attachments;
mod backstage;
pub mod calendar;
pub mod calendars;
pub mod categories;
mod certificate;
pub mod chat;
pub mod cleanup;
mod compose;
pub mod contacts;
mod date_picker;
mod dates;
pub mod eml;
pub mod folders;
mod fonts;
pub mod groups;
pub mod helder;
pub mod help;
mod html;
pub mod i18n;
pub mod importance;
pub mod install;
pub mod invitation;
pub mod junk;
pub mod meeting;
pub mod menubar;
pub mod message_menu;
pub mod notice;
pub mod notifications;
pub mod print;
pub mod quicksteps;
pub mod receipt;
pub mod release_notes;
pub mod reminders;
pub mod remote;
pub mod resend;
pub mod rich_edit;
pub mod rules;
pub mod search_folders;
mod signature;
pub mod tasks;
pub mod templates;
pub mod theme;
pub mod unsubscribe;
pub mod updates;
pub mod vcard;
pub mod view;
pub mod welcome;

use crate::i18n::tr;
use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};

use merel_engine::{AccountStatus, Engine};
use merel_model::{FolderKind, Warning};

pub use backstage::AccountForm;
pub use compose::{Compose, Kind as ComposeKind};
pub use notice::Notice;

pub use theme::{APP_COLOR, Tokens};

/// The four Outlook modules, in the order they are being built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Module {
    Mail,
    Calendar,
    Chat,
    People,
    Tasks,
}

impl Module {
    const ALL: [Module; 5] = [Module::Mail, Module::Calendar, Module::Chat, Module::People, Module::Tasks];

    fn label(self) -> String {
        tr(match self {
            Module::Mail => "module-mail",
            Module::Calendar => "module-calendar",
            Module::Chat => "module-chat",
            Module::People => "module-people",
            Module::Tasks => "module-tasks",
        })
    }

    fn icon(self) -> helder::Icon {
        match self {
            Module::Mail => helder::Icon::Mail,
            Module::Calendar => helder::Icon::Calendar,
            Module::Chat => helder::Icon::MessagesSquare,
            Module::People => helder::Icon::User,
            Module::Tasks => helder::Icon::CircleCheck,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    File,
    Home,
    Folder,
    SendReceive,
    View,
    Help,
}

impl Tab {
    const ALL: [Tab; 6] = [Tab::File, Tab::Home, Tab::Folder, Tab::SendReceive, Tab::View, Tab::Help];

    fn label(self) -> String {
        tr(match self {
            Tab::File => "tab-file",
            Tab::Home => "tab-home",
            Tab::Folder => "tab-folder",
            Tab::SendReceive => "tab-send-receive",
            Tab::View => "tab-view",
            Tab::Help => "tab-help",
        })
    }
}

pub struct MailApp {
    pub engine: Engine,
    pub module: Module,
    tab: Tab,
    form: AccountForm,
    pub compose: Option<Compose>,
    /// Messages on their way after their window closed, as Outlook's Postvak UIT.
    pub sending: Vec<compose::Sending>,
    pub calendar: calendar::State,
    pub contacts: contacts::State,
    pub tasks: tasks::State,
    pub chat: chat::State,
    /// What the last action has to say, shown at the top of the window (see [`notice`]).
    notice: Option<notice::Notice>,
    /// The message whose body is on screen; when it changes, its pictures leave the cache.
    shown_message: Option<String>,
    /// The message waiting to be marked read, and since when, for "Gelezen markeren: Na 5 seconden".
    read_timer: Option<(String, std::time::Instant)>,
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
    /// Whether the "Move to" window for the selected message is open.
    pub move_to: bool,
    /// The window for a new, renamed or removed folder, while it is open.
    pub folder_edit: Option<folders::FolderEdit>,
    /// The "Afmelden" question, while it is open.
    pub unsubscribe: Option<unsubscribe::Confirm>,
    /// The messages a read receipt went out for since the app started; the strip then says so.
    pub receipts_sent: std::collections::HashSet<String>,
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
    /// The check for a newer release, and the release it found.
    pub updates: updates::Updates,
    /// The step of the first-start welcome, while it is open.
    pub welcome: Option<welcome::Step>,
    /// The conversations (by key) unfolded in the list, while the conversation view is on.
    pub expanded: std::collections::HashSet<String>,
    /// Whether the saved theme has been handed to egui yet (on the first frame).
    theme_applied: bool,
    /// The window title last handed to egui, so it is only sent when it changes.
    title: Option<String>,
    /// Room kept free at the left of the top bar for the system's window buttons, when the app
    /// draws into the title bar itself (macOS); 0 when the system draws its own title bar.
    pub title_bar_inset: f32,
    /// Whether the ribbon's tabs live in the system's menu bar (macOS) instead of the window;
    /// the window then keeps one command bar, the module's Start tab, as Outlook for Mac.
    pub native_menu: bool,
    /// The menus for the menu bar, noted each frame while [`MailApp::native_menu`] is on.
    pub menu_bar: Vec<menubar::Menu>,
    /// A click in the menu bar, to run in the next frame: the menu's place and the item's path.
    menu_click: Option<(usize, menubar::Path)>,
    /// The computer's notifications and the count on the app icon; `None` leaves them out
    /// (tests, and until the app sets one up).
    pub notifier: Option<merel_notify::Notifier>,
    /// The last notifications handed to the computer, newest last (see [`notifications`]).
    pub alerts: Vec<merel_notify::Alert>,
    /// Which page Bestand shows.
    pub(crate) file_page: backstage::Page,
}

/// The window title: the app's name, with the unread inbox mail of every account in brackets
/// when there is any, as Outlook shows it.
pub fn window_title(inbox_unread: u32) -> String {
    if inbox_unread == 0 { "Merel".to_string() } else { format!("Merel ({inbox_unread})") }
}

/// A message being dragged from the list, to drop on a folder.
struct DraggedMessage(String);

/// What the reading pane says while several messages are selected.
pub(crate) fn multi_hint() -> String {
    tr("list-multi-hint")
}

/// One row of the message list: a message, indented when it sits inside an unfolded
/// conversation, with the fold control when it heads a conversation of several messages.
struct Row<'a> {
    m: &'a merel_model::MessageHeader,
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
    list: Vec<&'a merel_model::MessageHeader>,
    conversations: bool,
    expanded: &std::collections::HashSet<String>,
    selected: Option<&str>,
) -> Vec<Row<'a>> {
    if !conversations {
        return list.into_iter().map(|m| Row { m, indent: false, fold: None }).collect();
    }
    let mut rows = Vec::new();
    for c in merel_engine::view::conversations(list) {
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
        i18n::set(view.language.language());
        engine.session.relabel();
        engine.session.sort = view.sort;
        let mut app = MailApp {
            engine,
            module: Module::Mail,
            tab: Tab::Home,
            form: AccountForm::default(),
            compose: None,
            sending: Vec::new(),
            calendar: calendar::State::default(),
            contacts: contacts::State::default(),
            tasks: tasks::State::default(),
            chat: chat::State::default(),
            notice: None,
            unsubscribe: None,
            receipts_sent: Default::default(),
            cleanup: None,
            preview: None,
            message_window: false,
            search_folder_form: None,
            shown_message: None,
            read_timer: None,
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
            updates: updates::Updates::default(),
            welcome: None,
            expanded: std::collections::HashSet::new(),
            theme_applied: false,
            title_bar_inset: 0.0,
            native_menu: false,
            menu_bar: Vec::new(),
            menu_click: None,
            title: None,
            notifier: None,
            alerts: Vec::new(),
            file_page: backstage::Page::default(),
        };
        app.welcome = welcome::due(&app).then_some(welcome::Step::Hello);
        app
    }

    /// Runs a command; an error is shown, never raised. Deleting a message that is in the
    /// Deleted folder already would remove it for good, so that asks first.
    fn run(&mut self, id: &str, params: Value) {
        if id == "message.delete" && self.selected_is_deleted() {
            self.delete_forever();
            return;
        }
        match self.engine.run(id, &params) {
            // A warning or error about an earlier step is over once the next one works; good
            // news stays for its few seconds.
            Ok(()) => {
                if self.notice.as_ref().is_some_and(|n| matches!(n.tone, notice::Tone::Warning | notice::Tone::Error)) {
                    self.notice = None;
                }
            }
            Err(e) => {
                log::warn!("{id}: {e}");
                self.notice = Some(match e {
                    merel_engine::CommandError::NoSelection => Notice::warning(tr("notice-select-message")),
                    merel_engine::CommandError::NoFolder(_) => Notice::warning(tr("notice-no-folder")),
                    e => Notice::error(e.to_string()),
                });
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if !self.theme_applied {
            ui.ctx().set_fonts(fonts::definitions());
            theme::install(ui.ctx());
            ui.ctx().set_theme(self.view.theme.preference());
            self.theme_applied = true;
        }
        let t = Tokens::for_dark(ui.visuals().dark_mode);
        // Decoders for pictures embedded in mail; a no-op after the first frame.
        egui_extras::install_image_loaders(ui.ctx());
        self.engine.poll();
        self.wake_snoozed();
        self.tick_read_timer(ui.ctx());
        self.tick_updates(ui.ctx());
        let arrivals = self.engine.session.take_arrivals();
        if !arrivals.is_empty() {
            self.notice = Some(Notice::info(if arrivals.len() == 1 {
                crate::i18n::tr_args("arrived", &[("count", 1.into())])
            } else {
                crate::i18n::tr_args("arrived", &[("count", arrivals.len().into())])
            }));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
            self.alert_new_mail(&ui.ctx().clone(), &arrivals);
        }
        self.tick_notifications(&ui.ctx().clone());
        let hits = self.engine.session.take_rule_hits();
        if hits > 0 {
            self.notice = Some(Notice::info(rules::rule_notice(hits)));
        }
        if let Some(failure) = self.engine.session.take_scheduled_failures().pop() {
            self.notice = Some(Notice::error(failure));
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
        if self.native_menu {
            menu_bar(self, ui, &t);
        }
        status_bar(self, ui, &t);
        let ctx = ui.ctx().clone();
        compose::follow_sending(self);
        compose::show(self, &ctx, &t);
        calendar::window(self, &ctx, &t);
        calendar::goto_window(self, &ctx, &t);
        calendars::window(self, &ctx, &t);
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
        updates::window(self, &ctx, &t);
        welcome::window(self, &ctx, &t);
        drag_preview(self, &ctx, &t);
        calendar::drag_preview(self, &ctx, &t);
        helder::linger_closed_windows(&ctx);
        if self.tab == Tab::Home && self.module == Module::Mail {
            shortcuts(self, &ctx);
        } else if self.tab == Tab::Home {
            module_shortcuts(self, &ctx);
        }
        // What an action has to say floats just under the commands, where the person looks.
        notice::toast(self, &ctx, &t, ui.available_rect_before_wrap().top());
        if self.tab == Tab::File {
            backstage::show(self, ui, &t);
            return;
        }
        // The window's grey ground, which shows between the cards.
        ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, t.window);
        rail(self, ui, &t);
        if self.module == Module::Mail {
            folder_pane(self, ui, &t);
            // The list and the reading pane are cards on the window's ground, a gap between them.
            let frame = helder::card_frame(&t).outer_margin(Margin { left: 0, right: 6, top: 0, bottom: 12 });
            match self.view.reading_pane {
                view::ReadingPane::Right => {
                    egui::Panel::left("messages")
                        .default_size(360.0)
                        .resizable(true)
                        .show_separator_line(false)
                        .frame(frame)
                        .show(ui, |ui| message_list(self, ui, &t));
                }
                view::ReadingPane::Bottom => {
                    egui::Panel::top("messages")
                        .default_size(320.0)
                        .resizable(true)
                        .show_separator_line(false)
                        .frame(frame)
                        .show(ui, |ui| message_list(self, ui, &t));
                }
                view::ReadingPane::Off => {
                    // Outlook's "Leesvenster uit": the list takes the room and a message is read
                    // in its own window.
                    egui::CentralPanel::default().frame(frame).show(ui, |ui| message_list(self, ui, &t));
                }
            }
            if self.view.reading_pane != view::ReadingPane::Off {
                let frame = helder::card_frame(&t).inner_margin(Margin::same(20)).outer_margin(Margin { left: 6, right: 12, top: 0, bottom: 12 });
                egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                    reading_pane(self, ui, &t);
                });
            }
        } else if self.module == Module::Calendar {
            calendar::show(self, ui, &t);
        } else if self.module == Module::Chat {
            chat::show(self, ui, &t);
        } else if self.module == Module::People {
            contacts::show(self, ui, &t);
        } else {
            tasks::show(self, ui, &t);
        }
    }
}

/// The search box's widget id, so a shortcut can put the cursor there.
const SEARCH_BOX: &str = "message_search_box";

/// The button under the blocked-pictures notice that puts the sender on the safe senders list,
/// so their pictures load from then on without asking, as Outlook does for Safe Senders.
pub(crate) fn always_from_sender() -> String {
    tr("images-always-from-sender")
}
pub(crate) fn safe_images_notice() -> String {
    tr("images-safe-sender-notice")
}

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
    if app.module == Module::Calendar && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::G)) {
        app.open_goto();
        return;
    }
    let (box_id, query) = match app.module {
        Module::Mail => return,
        Module::Calendar => (calendar::SEARCH_BOX, &mut app.calendar.query),
        Module::Chat => (chat::SEARCH_BOX, &mut app.chat.query),
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
        } else if i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::F) {
            command = Some("forward_attached");
        } else if i.consume_key(Modifiers::COMMAND, Key::F) {
            compose = Some(ComposeKind::Forward);
        } else if i.consume_key(Modifiers::COMMAND, Key::P) {
            command = Some("message.print");
        } else if i.consume_key(Modifiers::COMMAND, Key::S) {
            command = Some("message.save_as");
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
        Some("forward_attached") if has => app.forward_as_attachment(),
        Some("message.save_as") => app.save_as_file(),
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
            app.select_message(&id);
        }
    }
}

/// The quick filters over the message list; the rest are in the list's filter menu.
const LIST_CHIPS: [merel_engine::view::Filter; 4] = [
    merel_engine::view::Filter::All,
    merel_engine::view::Filter::Unread,
    merel_engine::view::Filter::Flagged,
    merel_engine::view::Filter::Attachments,
];

/// The label of the small button that empties a search box.
pub(crate) fn search_clear() -> String {
    tr("search-clear")
}
/// The button in the search box that keeps the search as a search folder.
pub(crate) fn search_folder() -> String {
    tr("search-folder")
}
/// Where the search box looks, in its menu.
pub(crate) fn scope_folder() -> String {
    tr("search-scope-folder")
}
pub(crate) fn scope_all() -> String {
    tr("search-scope-all")
}
fn search_hint() -> String {
    tr("search-hint")
}
/// The height of the top bar's search box, and of the bar.
const SEARCH_HEIGHT: f32 = 30.0;
/// The height of the title bar the app draws itself on macOS, as tall as Outlook's; the window
/// buttons sit in its middle.
pub const TITLE_BAR_HEIGHT: f32 = 52.0;
/// The search box's height in that title bar.
const TITLE_SEARCH_HEIGHT: f32 = 32.0;

/// The filter menu's name over the message list.
pub(crate) fn list_filter() -> String {
    tr("list-filter")
}

/// The search bar at the top of the window, as the new Outlook has it: one field for the
/// mail, with the recent searches, "Alle mappen" and, while searching, the way to keep the
/// search as a search folder.
/// The bar at the top of the window: the search box, centred, as the Helder design and
/// Outlook have it. Where the app draws into the title bar (macOS) it is the title bar: it
/// keeps room for the window buttons and moves the window when dragged.
fn search_bar(app: &mut MailApp, ui: &mut Ui, t: &Tokens, search: bool) {
    let searching = app.engine.session.is_searching();
    let mut save_search = false;
    let mut remember: Option<String> = None;
    let mut recent_pick: Option<Option<String>> = None;
    let recents = app.view.recent_searches.clone();
    let inset = app.title_bar_inset;
    let title = app.title.clone().unwrap_or_else(|| "Merel".to_string());
    // In the macOS title bar the box sits in the middle of the tall bar, level with the window
    // buttons.
    let height = if inset > 0.0 { TITLE_SEARCH_HEIGHT } else { SEARCH_HEIGHT };
    let pad = ((TITLE_BAR_HEIGHT - TITLE_SEARCH_HEIGHT) / 2.0) as i8;
    egui::Panel::top("searchbar")
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(Margin::symmetric(12, if inset > 0.0 { pad } else { 4 })))
        .show(ui, |ui| {
            let bar = egui::Rect::from_min_size(ui.max_rect().min, vec2(ui.available_width(), height));
            if inset > 0.0 {
                // The bar is the title bar: dragging it moves the window, a double click zooms.
                let r = ui.interact(bar.expand2(vec2(12.0, f32::from(pad))), egui::Id::new("title-bar"), Sense::click_and_drag());
                if r.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if r.double_clicked() {
                    let zoomed = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!zoomed));
                }
            }
            let row = vec2(ui.available_width(), height);
            ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if !search {
                    ui.painter().text(bar.center(), egui::Align2::CENTER_CENTER, &title, egui::FontId::proportional(13.0), t.text_dim);
                    return;
                }
                // Centred in the window, clear of the window buttons.
                let full = ui.available_width() + 24.0;
                // About two fifths of the window, as Outlook's.
                let width = if inset > 0.0 {
                    (full * 0.42).clamp(360.0, 760.0).min(full - 2.0 * inset - 48.0).max(280.0)
                } else {
                    (full - 2.0 * inset - 48.0).clamp(320.0, 640.0)
                };
                ui.add_space(((full - width) / 2.0 - 12.0).max(inset));
                let session = &mut app.engine.session;
                let field = egui::Frame::new().fill(t.card).corner_radius(helder::CONTROL_RADIUS).inner_margin(Margin {
                    left: 10,
                    right: 2,
                    top: 0,
                    bottom: 0,
                });
                field.show(ui, |ui| {
                    ui.set_width(width - 12.0);
                    ui.set_height(height);
                    // What follows the words takes the room it took last frame.
                    let tail_id = egui::Id::new("search-tail");
                    let tail: f32 = ui.data(|d| d.get_temp(tail_id)).unwrap_or(120.0);
                    ui.horizontal_centered(|ui| {
                        ui.add(helder::Icon::Search.image(16.0, t.text_dim));
                        let box_r = ui
                            .add(
                                egui::TextEdit::singleline(&mut session.search)
                                    .id(egui::Id::new(SEARCH_BOX))
                                    .frame(egui::Frame::NONE)
                                    .hint_text(RichText::new(search_hint()).color(t.text_dim))
                                    .text_color(t.text)
                                    .desired_width((ui.available_width() - tail).max(80.0))
                                    .margin(Margin::symmetric(4, 4)),
                            )
                            .on_hover_text(merel_engine::search::HINT);
                        // Leaving the box (Enter, a click elsewhere) is when the words count as a search.
                        if box_r.lost_focus() && !session.search.trim().is_empty() {
                            remember = Some(session.search.clone());
                        }
                        let start = ui.cursor().min.x;
                        helder::quiet(ui, t, |ui| {
                            ui.spacing_mut().button_padding = vec2(6.0, 3.0);
                            if searching && box_button(ui, t, helder::Icon::X, &search_clear(), height - 4.0).clicked() {
                                session.search.clear();
                            }
                            if !recents.is_empty() {
                                // Outlook's "Recente zoekopdrachten": the last words typed, to search again.
                                let button = egui::Button::image(helder::Icon::Clock.image(helder::ICON, t.text))
                                    .frame_when_inactive(false)
                                    .corner_radius(helder::CONTROL_RADIUS)
                                    .min_size(vec2(height - 4.0, height - 4.0));
                                let r = egui::containers::menu::MenuButton::from_button(button)
                                    .ui(ui, |ui| {
                                        for q in &recents {
                                            if ui.button(q).clicked() {
                                                recent_pick = Some(Some(q.clone()));
                                                ui.close();
                                            }
                                        }
                                        ui.separator();
                                        if ui.button(view::recent_clear()).clicked() {
                                            recent_pick = Some(None);
                                            ui.close();
                                        }
                                    })
                                    .0;
                                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, view::recent_menu()));
                                r.on_hover_text(view::recent_menu());
                            }
                            if searching
                                && box_button(ui, t, helder::Icon::FolderPlus, &search_folder(), height - 4.0)
                                    .on_hover_text(tr("search-folder-hint"))
                                    .clicked()
                            {
                                save_search = true;
                            }
                            // Where to search, as Outlook's scope menu in its search box.
                            let scope = if session.search_all { &scope_all() } else { &scope_folder() };
                            let button = egui::Button::new(RichText::new(scope).size(13.0).color(t.text))
                                .right_text(helder::Icon::ChevronDown.image(14.0, t.text_dim))
                                .frame_when_inactive(false)
                                .corner_radius(helder::CONTROL_RADIUS)
                                .min_size(vec2(0.0, height - 4.0));
                            egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                                if ui.selectable_label(!session.search_all, scope_folder()).clicked() {
                                    session.search_all = false;
                                    ui.close();
                                }
                                if ui.selectable_label(session.search_all, scope_all()).on_hover_text(tr("search-all-hint")).clicked() {
                                    session.search_all = true;
                                    ui.close();
                                }
                            });
                        });
                        let used = ui.cursor().min.x - start + 2.0;
                        if (used - tail).abs() > 0.5 {
                            ui.data_mut(|d| d.insert_temp(tail_id, used));
                            ui.ctx().request_discard("the search box's buttons changed");
                        }
                    });
                });
            });
        });
    if save_search {
        app.ask_search_folder();
    }
    if let Some(q) = remember {
        app.view.remember_search(&q);
        let ctx = ui.ctx().clone();
        app.save_view(&ctx);
    }
    match recent_pick {
        Some(Some(q)) => {
            app.engine.session.search = q.clone();
            app.view.remember_search(&q);
            let ctx = ui.ctx().clone();
            app.save_view(&ctx);
        }
        Some(None) => {
            app.view.recent_searches.clear();
            let ctx = ui.ctx().clone();
            app.save_view(&ctx);
        }
        None => {}
    }
}

/// A small button with only an icon, sized to sit inside the search box.
fn box_button(ui: &mut Ui, t: &Tokens, icon: helder::Icon, label: &str, size: f32) -> egui::Response {
    let r = ui.add(
        egui::Button::image(icon.image(16.0, t.text)).frame_when_inactive(false).corner_radius(helder::CONTROL_RADIUS).min_size(vec2(size, size)),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    r.on_hover_text(label)
}

fn ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    // The File tab is a page of its own, as Outlook's: no search bar and no command bar.
    // With the tabs in the menu bar, the window keeps only the Start tab, as Outlook for Mac.
    if app.native_menu && app.tab != Tab::File {
        app.tab = Tab::Home;
    }
    let file = app.tab == Tab::File;
    let search = app.module == Module::Mail && !file;
    if search || app.title_bar_inset > 0.0 {
        search_bar(app, ui, t, search);
    }
    // The tabs line up with the first button on the ribbon card under them.
    let tabs_margin = Margin { left: 12, right: 12, top: 0, bottom: 2 };
    if !app.native_menu {
        egui::Panel::top("tabs").show_separator_line(false).frame(egui::Frame::NONE.fill(t.window).inner_margin(tabs_margin)).show(ui, |ui| {
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
    }
    if file {
        return;
    }
    // The Helder ribbon: one line of icon buttons on a card over the window's grey ground.
    egui::Panel::top("ribbon")
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(Margin { left: 8, right: 12, top: 0, bottom: 8 }))
        .show(ui, |ui| {
            helder::card_frame(t).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.x = RIBBON_SPACING;
                let row = vec2(ui.available_width(), helder::BUTTON_HEIGHT);
                ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    helder::quiet(ui, t, |ui| {
                        // The Beeld tab is full; a roomier padding would push it off a 1280 px window.
                        ui.spacing_mut().button_padding = vec2(5.0, 4.0);
                        match app.tab {
                            Tab::Home if app.module == Module::Calendar => calendar_ribbon(app, ui, t),
                            Tab::Home if app.module == Module::Chat => chat::ribbon(app, ui, t),
                            Tab::Home if app.module == Module::People => contacts::ribbon(app, ui, t),
                            Tab::Home if app.module == Module::Tasks => tasks::ribbon(app, ui, t),
                            Tab::Home => mail_ribbon(app, ui, t),
                            Tab::Folder if app.module == Module::Mail => folders::ribbon(app, ui, t),
                            Tab::View => view::ribbon(app, ui, t),
                            Tab::Help => help::ribbon(app, ui, t),
                            Tab::SendReceive => send_receive_ribbon(app, ui, t),
                            _ => {
                                ui.label(RichText::new(tr("ribbon-not-yet")).color(t.text_dim));
                            }
                        }
                    })
                });
            });
        });
}

impl MailApp {
    /// Hands a click in the menu bar to the next frame: `menu` is the menu's place in
    /// [`MailApp::menu_bar`], `path` the labels down to the item.
    pub fn click_menu(&mut self, menu: usize, path: menubar::Path) {
        self.menu_click = Some((menu, path));
    }
}

/// Notes the menus of the menu bar from the ribbon's tabs, and runs a click made in it.
fn menu_bar(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let click = app.menu_click.take();
    if click.is_some() {
        // What the click changed shows in the menus of the next frame.
        ui.ctx().request_repaint();
    }
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt("menu-bar").invisible());
    let ui = &mut child;
    let mut menus: Vec<menubar::Menu> = Vec::new();
    let mut add = |app: &mut MailApp, ui: &mut Ui, title: String, f: &dyn Fn(&mut MailApp, &mut Ui)| {
        let index = menus.len();
        let click = click.as_ref().filter(|(i, _)| *i == index).map(|(_, p)| p.clone());
        let entries = menubar::capture(click, || f(app, ui));
        menus.push(menubar::Menu { title, entries });
    };
    add(app, ui, tr("menu-file"), &|app, _| file_menu(app));
    let module_menu = tr(match app.module {
        Module::Mail => "menu-message",
        Module::Calendar => "menu-calendar",
        Module::Chat => "menu-chat",
        Module::People => "menu-people",
        Module::Tasks => "menu-tasks",
    });
    add(app, ui, module_menu, &|app, ui| match app.module {
        Module::Mail => mail_ribbon(app, ui, t),
        Module::Calendar => calendar_ribbon(app, ui, t),
        Module::Chat => chat::ribbon(app, ui, t),
        Module::People => contacts::ribbon(app, ui, t),
        Module::Tasks => tasks::ribbon(app, ui, t),
    });
    if app.module == Module::Mail {
        add(app, ui, tr("menu-folder"), &|app, ui| {
            folders::ribbon(app, ui, t);
            ribbon_separator(ui);
            send_receive_ribbon(app, ui, t);
        });
    }
    add(app, ui, tr("menu-view"), &|app, ui| {
        for m in Module::ALL {
            if menubar::item(&m.label(), true, Some(app.module == m)) {
                app.module = m;
            }
        }
        ribbon_separator(ui);
        view::ribbon(app, ui, t);
    });
    add(app, ui, tr("menu-help"), &|app, ui| help::ribbon(app, ui, t));
    app.menu_bar = menus;
}

/// The Bestand menu: something new in each module, then what the Bestand page holds.
fn file_menu(app: &mut MailApp) {
    if menubar::item(&tr("new-mail"), true, None) {
        app.open_compose(ComposeKind::New);
    }
    if menubar::item(&tr("new-appointment"), true, None) {
        app.module = Module::Calendar;
        let date = app.calendar.focus;
        app.new_appointment(date, 9);
    }
    if menubar::item(&tr("new-contact"), true, None) {
        app.module = Module::People;
        app.new_contact();
    }
    if menubar::item(&tr("new-task"), true, None) {
        app.module = Module::Tasks;
        app.new_task();
    }
    menubar::separator();
    if menubar::item(&tr("menu-accounts"), true, None) {
        app.tab = Tab::File;
        app.file_page = backstage::Page::Accounts;
    }
    if menubar::item(&format!("{}…", notifications::page_label()), true, None) {
        app.tab = Tab::File;
        app.file_page = backstage::Page::Notifications;
    }
    menubar::separator();
    for action in backstage::FileAction::ALL {
        if menubar::item(&action.label(), true, None) {
            backstage::run(app, action);
        }
    }
}

/// The mail module's Start tab: new, respond, clean up, file, mark, categorise, quick steps.
fn mail_ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let has = app.engine.session.selected_message.is_some();
    let mut open: Option<ComposeKind> = None;
    view::group(ui, t, &tr("group-new"), |ui| {
        if ribbon_action_hint(ui, t, "✉", &tr("new-mail"), true, Some(&key_hint("N"))) {
            open = Some(ComposeKind::New);
        }
    });
    ribbon_separator(ui);
    view::group(ui, t, &tr("group-respond"), |ui| {
        if ribbon_action_hint(ui, t, "↩", &tr("mail-reply"), has, Some(&key_hint("R"))) {
            open = Some(ComposeKind::Reply);
        }
        if ribbon_action_hint(ui, t, "↞", &tr("mail-reply-all"), has, Some(&key_hint("Shift+R"))) {
            open = Some(ComposeKind::ReplyAll);
        }
        if ribbon_action_hint(ui, t, "↪", &tr("mail-forward"), has, Some(&key_hint("F"))) {
            open = Some(ComposeKind::Forward);
        }
    });
    if let Some(kind) = open {
        app.open_compose(kind);
    }
    ribbon_separator(ui);
    view::group(ui, t, &tr("group-clean"), |ui| {
        ribbon_button(app, ui, t, "🗑", &tr("delete"), ("message.delete", Some("Delete")), has);
        ribbon_button(
            app,
            ui,
            t,
            "🗄",
            &tr("mail-archive"),
            ("message.archive", Some(if cfg!(target_os = "macos") { "Ctrl+E" } else { "Backspace" })),
            has,
        );
        ribbon_button(app, ui, t, "⚠", &tr("mail-junk"), ("message.junk", None), has);
    });
    ribbon_separator(ui);
    view::group(ui, t, &tr("group-file"), |ui| {
        if ribbon_action_hint(ui, t, "📂", &tr("mail-move"), has, Some(&key_hint("Shift+V"))) {
            app.move_to = true;
        }
        if ribbon_action_hint(ui, t, "⚙", &tr("mail-rules"), true, Some(&tr("mail-rules-hint"))) {
            app.open_rules(false);
        }
    });
    ribbon_separator(ui);
    view::group(ui, t, &tr("group-tags"), |ui| {
        ribbon_button(app, ui, t, "◉", &tr("mail-read"), ("message.mark_read", Some(&key_hint("Q"))), has);
        ribbon_button(app, ui, t, "○", &tr("mail-unread"), ("message.mark_unread", Some(&key_hint("U"))), has);
        ribbon_button(app, ui, t, "⚑", &tr("mail-flag"), ("message.flag", None), has);
    });
    if !menubar::capturing() {
        categories::menu(app, ui, t);
    }
    ribbon_separator(ui);
    quicksteps::ribbon(app, ui, t);
}

/// The Verzenden/ontvangen tab.
fn send_receive_ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let can = !app.engine.session.is_sample() && app.engine.session.selected_folder.is_some();
    ribbon_button(app, ui, t, "⟳", &tr("refresh"), ("folder.refresh", Some("F5")), can);
}

fn calendar_ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    use calendar::View;
    if ribbon_action_hint(ui, t, "📅", &tr("new-appointment"), true, Some(&key_hint("Shift+A"))) {
        let date = app.calendar.focus;
        app.new_appointment(date, 9);
    }
    ribbon_separator(ui);
    if ribbon_action(ui, t, "◎", &tr("calendar-today"), true) {
        app.calendar.focus = calendar::today();
    }
    if ribbon_action_hint(ui, t, "↪", &tr("calendar-goto"), true, Some(&key_hint("G"))) {
        app.open_goto();
    }
    ribbon_separator(ui);
    for (glyph, label, view) in [
        ("▯", "calendar-day", View::Day),
        ("▤", "calendar-work-week", View::WorkWeek),
        ("▥", "calendar-week", View::Week),
        ("▦", "calendar-month", View::Month),
    ] {
        if ribbon_toggle(ui, t, glyph, &tr(label), app.calendar.view == view) {
            app.calendar.view = view;
        }
    }
    ribbon_separator(ui);
    let has = app.engine.session.selected_appointment.is_some();
    if ribbon_action(ui, t, "🗑", &tr("delete"), has) {
        app.run("appointment.delete", Value::Null);
        app.calendar.editor = None;
    }
    if ribbon_action(ui, t, "⟳", &tr("refresh"), !app.engine.session.item_accounts.is_empty()) {
        app.run("sync.refresh", Value::Null);
    }
}

/// "Cmd+N" on a Mac, "Ctrl+N" elsewhere.
pub(crate) fn key_hint(key: &str) -> String {
    let modifier = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
    format!("{modifier}+{key}")
}

/// The least width of a ribbon button; a longer label makes its button wider.
const RIBBON_BUTTON_WIDTH: f32 = 36.0;
/// The space between a ribbon button's icon and its label.
const RIBBON_ICON_GAP: f32 = 6.0;
/// The space between ribbon buttons and groups; egui's default of 8 wastes the width of a
/// 1280 px window.
const RIBBON_SPACING: f32 = 4.0;
/// Room left and right of a ribbon button's label.
const RIBBON_LABEL_PADDING: f32 = 6.0;
/// The font of a ribbon button's label.
fn ribbon_label_font() -> egui::FontId {
    egui::FontId::proportional(13.0)
}

/// A large ribbon button: glyph above label, drawn by hand as one widget so a screen reader
/// sees a single button named `label`. Returns true when clicked.
pub(crate) fn ribbon_action(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, enabled: bool) -> bool {
    ribbon_action_hint(ui, t, glyph, label, enabled, None)
}

/// [`ribbon_action`] with the keyboard shortcut shown on hover. In the Helder style a ribbon
/// button is one line: a line icon and its label; a label without an icon keeps its glyph.
fn ribbon_action_hint(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, enabled: bool, shortcut: Option<&str>) -> bool {
    ribbon_draw(ui, t, glyph, label, enabled, shortcut, None)
}

/// A ribbon button that stays lit while `on`, such as the calendar's current view.
fn ribbon_toggle(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, on: bool) -> bool {
    ribbon_draw(ui, t, glyph, label, true, None, Some(on))
}

/// [`ribbon_toggle`] with `hint` shown on hover.
fn ribbon_toggle_hint(ui: &mut Ui, t: &Tokens, label: &str, on: bool, hint: &str) -> bool {
    if menubar::capturing() {
        return menubar::item(label, true, Some(on));
    }
    ribbon_paint(ui, t, "", label, true, None, Some(on)).on_hover_text(hint).clicked()
}

/// Draws a ribbon button; `on` is set for a button that can stay lit. "Nieuwe e-mail" and the
/// other "new" buttons are the blue main button of their ribbon.
fn ribbon_draw(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, enabled: bool, shortcut: Option<&str>, on: Option<bool>) -> bool {
    if menubar::capturing() {
        return menubar::item(label, enabled, on);
    }
    let r = ribbon_paint(ui, t, glyph, label, enabled, shortcut, on);
    enabled && r.clicked()
}

/// Draws a ribbon button and gives its response.
fn ribbon_paint(ui: &mut Ui, t: &Tokens, glyph: &str, label: &str, enabled: bool, shortcut: Option<&str>, on: Option<bool>) -> egui::Response {
    let primary = enabled && helder::ribbon_primary(label);
    let lit = on == Some(true);
    let icon = helder::ribbon_icon(label, glyph);
    // Like the simplified ribbon of the new Outlook, the everyday actions show only their icon.
    let icon_only = icon.is_some() && helder::ribbon_icon_only(label);
    let label_width = if icon_only { 0.0 } else { ui.painter().layout_no_wrap(label.to_owned(), ribbon_label_font(), Color32::WHITE).size().x };
    let lead =
        if icon.is_some() { helder::ICON } else { ui.painter().layout_no_wrap(glyph.to_owned(), ribbon_label_font(), Color32::WHITE).size().x };
    let gap = if icon_only { 0.0 } else { RIBBON_ICON_GAP };
    let width = (lead + gap + label_width + 2.0 * RIBBON_LABEL_PADDING).max(RIBBON_BUTTON_WIDTH);
    let (rect, r) = ui.allocate_exact_size(vec2(width, helder::BUTTON_HEIGHT), if enabled { Sense::click() } else { Sense::hover() });
    r.widget_info(|| match on {
        Some(on) => egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, label),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label),
    });
    let r = match shortcut {
        Some(s) => r.on_hover_text(format!("{label} ({s})")),
        None if icon_only => r.on_hover_text(label),
        None => r,
    };
    let color = if primary {
        t.on_accent
    } else if lit {
        t.accent_text
    } else if enabled {
        t.text
    } else {
        t.text_disabled
    };
    let fill = if primary {
        Some(t.accent)
    } else if lit {
        Some(t.checked)
    } else if r.hovered() && enabled {
        Some(t.hover)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, CornerRadius::same(helder::CONTROL_RADIUS), fill);
    }
    let left = if icon_only { rect.center().x - helder::ICON / 2.0 } else { rect.left() + RIBBON_LABEL_PADDING };
    let lead_color = if primary || lit || !enabled { color } else { t.accent_text };
    match icon {
        Some(icon) => icon
            .image(helder::ICON, lead_color)
            .paint_at(ui, egui::Rect::from_min_size(egui::pos2(left, rect.center().y - helder::ICON / 2.0), vec2(helder::ICON, helder::ICON))),
        None => {
            ui.painter().text(egui::pos2(left, rect.center().y), egui::Align2::LEFT_CENTER, glyph, ribbon_label_font(), lead_color);
        }
    }
    if !icon_only {
        ui.painter().text(egui::pos2(left + lead + gap, rect.center().y), egui::Align2::LEFT_CENTER, label, ribbon_label_font(), color);
    }
    r
}

/// The upright line between groups of ribbon buttons; a line in a menu of the menu bar.
pub(crate) fn ribbon_separator(ui: &mut Ui) {
    if menubar::capturing() {
        menubar::separator();
    } else {
        ui.separator();
    }
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
    egui::Panel::left("rail")
        .exact_size(60.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(Margin::symmetric(6, 4)))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                for m in Module::ALL {
                    let on = app.module == m;
                    let b = egui::Button::image(m.icon().image(22.0, if on { t.accent_text } else { t.text_dim }))
                        .fill(if on { t.checked } else { Color32::TRANSPARENT })
                        .frame_when_inactive(on)
                        .stroke(Stroke::NONE)
                        .corner_radius(10)
                        .min_size(vec2(48.0, 48.0));
                    let r = ui.add(b);
                    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, m.label()));
                    if r.on_hover_text(m.label()).clicked() {
                        app.module = m;
                    }
                    ui.add_space(4.0);
                }
            });
        });
}

/// The line icon of a folder in the folder pane.
fn folder_line_icon(kind: FolderKind) -> helder::Icon {
    match kind {
        FolderKind::Inbox => helder::Icon::Inbox,
        FolderKind::Drafts => helder::Icon::FilePen,
        FolderKind::Sent => helder::Icon::Send,
        FolderKind::Archive => helder::Icon::Archive,
        FolderKind::Junk => helder::Icon::ShieldAlert,
        FolderKind::Deleted => helder::Icon::Trash,
        FolderKind::Other => helder::Icon::Folder,
    }
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

/// How a folder row sits in the folder pane, beyond the folder itself.
#[derive(Default)]
struct RowPlace<'a> {
    /// Shown on hover, when there is one.
    hint: Option<String>,
    /// What a screen reader says instead of the name, when the name alone would not tell the
    /// row apart.
    label: Option<&'a str>,
    /// How many folders it sits in: each level moves it a step to the right.
    depth: usize,
    /// For a folder with subfolders, whether they are shown; it then gets a chevron.
    open: Option<bool>,
}

/// The step a subfolder moves to the right of its parent.
const FOLDER_INDENT: f32 = 16.0;

/// One row of the folder pane: chevron, icon, name and the unread count, highlighted when it
/// is the open folder. Also says whether the chevron was clicked.
fn folder_row(ui: &mut Ui, t: &Tokens, f: &merel_model::Folder, on: bool, place: RowPlace) -> (egui::Response, bool) {
    let RowPlace { hint, label, depth, open } = place;
    let r = ui.add_sized(
        [ui.available_width(), 28.0],
        egui::Button::new("")
            .fill(if on { t.checked } else { Color32::TRANSPARENT })
            .frame_when_inactive(on)
            .stroke(Stroke::NONE)
            .corner_radius(helder::CONTROL_RADIUS),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.unwrap_or(&f.name)));
    let r = match hint {
        Some(h) => r.on_hover_text(h),
        None => r,
    };
    let rect = r.rect.shrink2(vec2(8.0, 0.0));
    let left = rect.left() + FOLDER_INDENT * depth.min(6) as f32;
    let font = egui::FontId::proportional(13.5);
    // The chevron in front folds the subfolders open and shut, apart from opening the folder.
    let mut toggled = false;
    if let Some(open) = open {
        let zone = egui::Rect::from_center_size(egui::pos2(left + 4.0, rect.center().y), vec2(18.0, 24.0));
        let c = ui.interact(zone, ui.id().with(("fold", &f.id)), Sense::click());
        let what = tr(if open { "fold-close" } else { "fold-open" });
        c.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{what}: {}", f.name)));
        toggled = c.on_hover_text(what).clicked();
        let chevron = egui::Rect::from_center_size(zone.center(), vec2(12.0, 12.0));
        let icon = if open { helder::Icon::ChevronDown } else { helder::Icon::ChevronRight };
        icon.image(12.0, t.text_dim).paint_at(ui, chevron);
    }
    let icon = egui::Rect::from_center_size(egui::pos2(left + 22.0, rect.center().y), vec2(16.0, 16.0));
    folder_line_icon(f.kind).image(16.0, if on { t.accent_text } else { t.text_dim }).paint_at(ui, icon);
    let p = ui.painter();
    p.text(egui::pos2(left + 40.0, rect.center().y), egui::Align2::LEFT_CENTER, &f.name, font.clone(), t.text);
    if f.unread > 0 {
        p.text(rect.right_center(), egui::Align2::RIGHT_CENTER, f.unread.to_string(), font, t.accent_text);
    }
    // A message dragged from the list lands here.
    if r.dnd_hover_payload::<DraggedMessage>().is_some() {
        p.rect_stroke(r.rect, CornerRadius::same(4), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    (r, toggled)
}

/// The folders of one account as the folder pane shows them: each folder followed by its
/// subfolders, a step further in, while it is folded open (`open`). A folder whose parent is
/// not listed stands at the top. Each comes with its depth and, when it has subfolders,
/// whether they are shown.
fn folder_tree(folders: &[merel_model::Folder], open: &[String]) -> Vec<(merel_model::Folder, usize, Option<bool>)> {
    let ids: std::collections::HashSet<&str> = folders.iter().map(|f| f.id.as_str()).collect();
    let top = |f: &merel_model::Folder| f.parent.as_deref().is_none_or(|p| !ids.contains(p));
    let mut out = Vec::new();
    // Depth first, with the list's own order (special folders first, then by name) kept.
    let mut stack: Vec<(&merel_model::Folder, usize)> = folders.iter().filter(|f| top(f)).rev().map(|f| (f, 0)).collect();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    while let Some((f, depth)) = stack.pop() {
        if !seen.insert(f.id.as_str()) {
            continue;
        }
        let children: Vec<&merel_model::Folder> = folders.iter().filter(|c| c.parent.as_deref() == Some(f.id.as_str())).collect();
        let shown = open.contains(&f.id);
        out.push((f.clone(), depth, (!children.is_empty()).then_some(shown)));
        if shown {
            stack.extend(children.into_iter().rev().map(|c| (c, depth + 1)));
        }
    }
    out
}

fn folder_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("folders")
        .default_size(220.0)
        .resizable(true)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(Margin { left: 4, right: 10, top: 4, bottom: 10 }))
        .show(ui, |ui| {
            // Several accounts with all their folders run past the window: the pane scrolls.
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| folder_list(app, ui, t));
        });
}

fn folder_list(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let mut pick = None;
    let mut pick_drafts = None;
    let mut drop_on: Option<(String, String)> = None;
    let mut menu_on: Option<(egui::Response, String, FolderKind)> = None;
    let mut open_accounts = false;
    let mut unified_read = None;
    // Outlook's "Favorieten": the pinned folders of every account, above the accounts.
    let favorites: Vec<merel_model::Folder> = app.view.favorites.iter().filter_map(|id| app.engine.session.folder(id).cloned()).collect();
    if !favorites.is_empty() {
        ui.label(RichText::new(tr("folders-favorites")).size(12.0).strong().color(t.text_dim));
        ui.add_space(4.0);
        for f in &favorites {
            let on = app.engine.session.selected_folder.as_deref() == Some(f.id.as_str());
            let account = app.engine.session.accounts.iter().find(|a| a.id == f.account_id).map(|a| a.address.email().to_string());
            let (r, _) = folder_row(
                ui,
                t,
                f,
                on,
                RowPlace {
                    hint: account.map(|a| crate::i18n::tr_args("folder-of-account", &[("folder", f.name.as_str().into()), ("account", a.into())])),
                    ..RowPlace::default()
                },
            );
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
    // Outlook's "Alle accounts": each special folder once, with the mail of every
    // account in it, as long as there is more than one account.
    let unified = app.engine.session.unified_folders();
    if !unified.is_empty() {
        if folding_heading(ui, t, merel_engine::unified::HEADING, !app.view.all_accounts_closed, None).clicked() {
            app.view.all_accounts_closed = !app.view.all_accounts_closed;
            app.save_view(ui.ctx());
        }
        if !app.view.all_accounts_closed {
            ui.add_space(4.0);
            for f in &unified {
                let on = app.engine.session.selected_folder.as_deref() == Some(f.id.as_str());
                let label = crate::i18n::tr_args("folder-all-accounts", &[("folder", f.name.as_str().into())]);
                let hint = Some(crate::i18n::tr_args("folder-all-accounts-hint", &[("folder", f.name.as_str().into())]));
                let (r, _) = folder_row(ui, t, f, on, RowPlace { hint, label: Some(&label), ..RowPlace::default() });
                if let Some(dragged) = r.dnd_release_payload::<DraggedMessage>()
                    && let Some(target) = app.engine.session.drop_target(&f.id, &dragged.0)
                {
                    drop_on = Some((dragged.0.clone(), target));
                }
                if r.clicked() {
                    pick = Some(f.id.clone());
                }
                r.context_menu(|ui| {
                    if ui.button(tr("folders-mark-all-read")).clicked() {
                        unified_read = Some(f.id.clone());
                        ui.close();
                    }
                });
            }
        }
        ui.add_space(12.0);
    }
    let mut fold_account = None;
    let mut fold_folder = None;
    for account in &app.engine.session.accounts {
        // Each account under a heading that folds its folders shut: its name and address, as
        // the design has it, and a dot for the connection.
        let (name, email) = (account.address.display(), account.address.email());
        let heading = if name == email { email.to_string() } else { format!("{name} · {email}") };
        let (color, status) = match app.engine.session.status.get(&account.id) {
            Some(AccountStatus::Online) => (t.accent, tr("account-online")),
            Some(AccountStatus::Connecting) => (t.text_dim, tr("account-connecting")),
            Some(AccountStatus::Error(_)) => (t.flag, tr("account-error")),
            _ => (t.border, tr("account-offline")),
        };
        let open = !app.view.closed_accounts.contains(&account.id);
        if folding_heading(ui, t, &heading, open, Some((color, status.as_str()))).clicked() {
            fold_account = Some(account.id.clone());
        }
        if !open {
            ui.add_space(4.0);
            continue;
        }
        let folders: Vec<merel_model::Folder> = app.engine.session.folders.iter().filter(|f| f.account_id == account.id).cloned().collect();
        for (f, depth, shown) in &folder_tree(&folders, &app.view.open_folders) {
            let on = app.engine.session.selected_folder.as_deref() == Some(f.id.as_str());
            let (r, toggled) = folder_row(ui, t, f, on, RowPlace { depth: *depth, open: *shown, ..RowPlace::default() });
            if toggled {
                fold_folder = Some(f.id.clone());
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
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("drafts-local")));
            let r = r.on_hover_text(tr("drafts-local-hint"));
            let rect = r.rect.shrink2(vec2(8.0, 0.0));
            let p = ui.painter();
            let font = egui::FontId::proportional(13.5);
            p.text(rect.left_center(), egui::Align2::LEFT_CENTER, tr("drafts-local-row"), font.clone(), t.text);
            p.text(rect.right_center(), egui::Align2::RIGHT_CENTER, drafts.to_string(), font, t.text_dim);
            if r.clicked() {
                pick_drafts = Some(account.id.clone());
            }
        }
    }
    ui.add_space(12.0);
    // Outlook's "Zoekmappen": the saved searches, under the accounts.
    search_folders::section(app, ui, t);
    if ui.add(egui::Button::new(RichText::new(tr("account-add")).color(t.accent_text)).frame(false)).clicked() {
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
    if let Some(id) = unified_read {
        app.run("folder.mark_all_read", json!({ "id": id }));
    }
    if let Some(id) = fold_account {
        view::ViewPrefs::toggle(&mut app.view.closed_accounts, &id);
        app.save_view(ui.ctx());
    }
    if let Some(id) = fold_folder {
        view::ViewPrefs::toggle(&mut app.view.open_folders, &id);
        app.save_view(ui.ctx());
    }
}

/// The heading of a group in the folder pane that folds open and shut, with a chevron that
/// says which it is; `status` adds a coloured dot after the text, with what it means.
fn folding_heading(ui: &mut Ui, t: &Tokens, text: &str, open: bool, status: Option<(Color32, &str)>) -> egui::Response {
    let r = ui.add_sized([ui.available_width(), 26.0], egui::Button::new("").frame(false));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, text));
    let what = tr(if open { "fold-close" } else { "fold-open" });
    let r = match status {
        Some((_, tip)) => r.on_hover_text(format!("{tip}. {what}.")),
        None => r.on_hover_text(what),
    };
    let rect = r.rect.shrink2(vec2(8.0, 0.0));
    let chevron = egui::Rect::from_center_size(egui::pos2(rect.left() + 6.0, rect.center().y), vec2(14.0, 14.0));
    let icon = if open { helder::Icon::ChevronDown } else { helder::Icon::ChevronRight };
    icon.image(14.0, t.text_dim).paint_at(ui, chevron);
    // Small and quiet, as the design has the account headings.
    // A long name and address end in "…" so the dot after them stays in sight.
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(12.0), t.text_dim);
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 20.0 - if status.is_some() { 18.0 } else { 0.0 });
    let galley = ui.painter().layout_job(job);
    let pos = egui::pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0);
    let right = pos.x + galley.size().x;
    ui.painter().galley(pos, galley, t.text_dim);
    if let Some((dot, _)) = status {
        ui.painter().circle_filled(egui::pos2(right + 10.0, rect.center().y), 4.0, dot);
    }
    r
}

/// The "Move to" window: the folders of the selected message's account.
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
    if crate::helder::popup(ctx, t, "move-to", &tr("move-to-title"), 280.0, |ui| {
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
        if ui.button(tr("cancel")).clicked() {
            close = true;
        }
    }) {
        close = true;
    }
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
    let drafts: Vec<merel_model::StoredDraft> = app.engine.session.drafts_of(account_id).into_iter().cloned().collect();
    egui::Frame::NONE.inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.label(RichText::new(crate::i18n::tr_args("drafts-title", &[("count", drafts.len().into())])).size(18.0).strong().color(t.text));
        ui.label(RichText::new(tr("drafts-page-hint")).size(12.0).color(t.text_dim));
    });
    let mut open = None;
    let mut delete = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        if drafts.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(tr("drafts-none")).color(t.text_dim)));
        }
        for d in &drafts {
            egui::Frame::NONE.inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let no_subject = tr("no-subject");
                let subject = if d.subject.is_empty() { no_subject.as_str() } else { d.subject.as_str() };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(clip(subject, 48)).strong().color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match d.send_at {
                        Some(ts) => {
                            ui.label(
                                RichText::new(crate::i18n::tr_args("drafts-send-at", &[("when", compose::when_text(ts).into())]))
                                    .size(12.0)
                                    .color(t.accent_text),
                            )
                            .on_hover_text(tr("drafts-scheduled-hint"));
                        }
                        None => {
                            ui.label(RichText::new(dates::short(d.updated)).size(12.0).color(t.text_dim));
                        }
                    });
                });
                let no_recipient = tr("drafts-no-recipient");
                let to = if d.to.trim().is_empty() { no_recipient.as_str() } else { d.to.as_str() };
                ui.label(RichText::new(crate::i18n::tr_args("to-line", &[("to", clip(to, 56).into())])).size(12.0).color(t.text_dim));
                let preview: String = d.body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
                if !preview.is_empty() {
                    ui.label(RichText::new(clip(&preview, 72)).size(12.0).color(t.text_dim));
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("open")).clicked() {
                        open = Some(d.id.clone());
                    }
                    if ui.button(tr("drafts-delete")).clicked() {
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
            let folder_name = app.engine.session.selected_folder.as_deref().and_then(|id| app.engine.session.folder_name(id)).unwrap_or_default();
            let searching = app.engine.session.is_searching();
            let count = if searching { Some(app.engine.session.visible_messages().len()) } else { None };
            let mut server_search = false;
            let mut filter_pick: Option<merel_engine::view::Filter> = None;
            egui::Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 12, bottom: 6 }).show(ui, |ui| {
                let title = match count {
                    Some(n) => crate::i18n::tr_args("search-results", &[("count", n.into())]),
                    None => folder_name,
                };
                let current = app.engine.session.filter;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).size(18.0).strong().color(t.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // The filters that do not fit as chips, with the order of the list on the Beeld tab.
                        helder::quiet(ui, t, |ui| {
                            let button = egui::Button::image(helder::Icon::ListFilter.image(helder::ICON, t.text))
                                .frame_when_inactive(false)
                                .corner_radius(helder::CONTROL_RADIUS)
                                .min_size(vec2(32.0, 32.0));
                            let (r, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                                for f in merel_engine::view::Filter::ALL {
                                    if ui.selectable_label(current == f, filter_name(f)).clicked() {
                                        filter_pick = Some(f);
                                        ui.close();
                                    }
                                }
                            });
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, list_filter()));
                            r.on_hover_text(tr("list-filter-hint"));
                        });
                    });
                });
                ui.add_space(4.0);
                // Outlook's quick filters, as chips over the list.
                helder::quiet(ui, t, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for f in LIST_CHIPS {
                            if helder::chip(ui, t, &filter_name(f), current == f).clicked() {
                                filter_pick = Some(f);
                            }
                        }
                    });
                });
                // The server can look where Merel has not: older mail, and the text itself.
                let session = &app.engine.session;
                if session.can_search_server() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let busy = session.searching_server();
                        match session.current_hits() {
                            Some(hits) => {
                                let n = hits.ids.len();
                                ui.label(
                                    RichText::new(crate::i18n::tr_args("search-server-found", &[("count", n.into())])).size(12.0).color(t.text_dim),
                                );
                            }
                            None => {
                                let label = tr(if busy { "search-server-busy" } else { "search-server" });
                                if ui.add_enabled(!busy, egui::Button::new(label)).on_hover_text(tr("search-server-hint")).clicked() {
                                    server_search = true;
                                }
                            }
                        }
                    });
                    if let Some(e) = &session.search_error {
                        ui.label(
                            RichText::new(crate::i18n::tr_args("search-server-failed", &[("error", e.to_string().into())])).size(12.0).color(t.flag),
                        );
                    }
                }
            });
            if let Some(f) = filter_pick {
                app.run("view.filter", json!({ "filter": f }));
            }
            if server_search {
                let _ = app.engine.run("search.server", &serde_json::Value::Null);
            }
            let mut pick = None;
            let mut older = false;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let list = app.engine.session.visible_messages();
                let loading = app.engine.session.selected_folder.as_deref().is_some_and(|f| app.engine.session.folder_loading(f));
                if list.is_empty() {
                    ui.add_space(24.0);
                    let msg = if searching && app.engine.session.current_hits().is_some() {
                        tr("list-none-found-server")
                    } else if searching {
                        tr("list-none-found")
                    } else if loading {
                        tr("list-fetching")
                    } else {
                        tr("list-empty")
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
                let mut menu_pick: Option<(merel_model::MessageHeader, message_menu::Pick)> = None;
                let mut open_window: Option<String> = None;
                // Date groups, as Outlook heads the list, while the list is in date order.
                let grouped =
                    app.view.groups && matches!(app.engine.session.sort, merel_engine::view::Sort::Newest | merel_engine::view::Sort::Oldest);
                let today = chrono::Local::now().date_naive();
                let now_ts = chrono::Local::now().timestamp();
                let mut last_group: Option<String> = None;
                for row in rows {
                    let m = row.m;
                    if grouped && !row.indent {
                        let group = dates::date_group(m.date, today);
                        if last_group.as_ref() != Some(&group) {
                            group_heading(ui, t, &group);
                            last_group = Some(group);
                        }
                    }
                    let on = selected.as_deref() == Some(m.id.as_str()) || app.engine.session.marked.contains(&m.id);
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click_and_drag());
                    r.dnd_set_drag_payload(DraggedMessage(m.id.clone()));
                    let hover = match app.engine.session.follow_up(&m.id).and_then(merel_engine::followup::due_day) {
                        Some(d) => format!("{}\n{}", m.subject, crate::i18n::tr_args("follow-up-before", &[("day", dates::day_name(d).into())])),
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
                    let snoozed = app.engine.session.snoozed_until(m, now_ts).is_some();
                    if let Some(p) = message_menu::menu(&r, m, known_sender, in_junk, ignored, snoozed) {
                        menu_pick = Some((m.clone(), p));
                    }
                    // The fold control is its own widget, drawn over the row's left edge.
                    if let Some(fold) = &row.fold {
                        let box_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 16.0, rect.top() + 14.0), vec2(18.0, 18.0));
                        let c = ui.interact(box_rect, ui.make_persistent_id(("fold", &m.id)), Sense::click());
                        let label = tr(if fold.open { "conversation-close" } else { "conversation-open" });
                        c.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
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
                    // Each row a soft rounded block, as the design draws them; unread ones keep
                    // their blue bar at the left.
                    let block = rect.shrink2(vec2(6.0, 2.0));
                    p.rect_filled(block, CornerRadius::same(helder::CONTROL_RADIUS), fill);
                    if on {
                        p.rect_stroke(block, CornerRadius::same(helder::CONTROL_RADIUS), Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
                    }
                    if !m.flags.seen {
                        let bar = egui::Rect::from_min_size(block.min + vec2(0.0, 8.0), vec2(3.0, block.height() - 16.0));
                        p.rect_filled(bar, CornerRadius::same(2), t.accent);
                    }
                    let shift = if row.indent {
                        32.0
                    } else if row.fold.is_some() {
                        16.0
                    } else {
                        0.0
                    };
                    // The sender's avatar, left of the three lines.
                    let avatar = if compact { 0.0 } else { 44.0 };
                    if !compact {
                        helder::avatar(p, egui::pos2(rect.left() + 32.0 + shift, rect.top() + 26.0), 16.0, m.from.display(), m.from.email());
                    }
                    let x = rect.left() + 14.0 + shift + avatar;
                    let w = rect.width() - 28.0 - shift - avatar;
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
                        clip(&m.subject, if squares > 0.0 { 34 } else { 42 }),
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
                            clip(&preview, 48),
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
                        app.select_message(&id);
                    }
                    app.message_window = true;
                }
                let selected = app.engine.session.selected_folder.clone();
                if !searching && let Some(folder) = selected {
                    if loading && !app.engine.session.visible_messages().is_empty() {
                        ui.add_space(10.0);
                        ui.vertical_centered(|ui| ui.label(RichText::new(tr("list-fetching")).size(12.0).color(t.text_dim)));
                        ui.add_space(10.0);
                    } else if app.engine.session.can_load_older(&folder) {
                        ui.add_space(10.0);
                        ui.vertical_centered(|ui| {
                            if ui.button(tr("list-load-older")).on_hover_text(tr("list-load-older-hint")).clicked() {
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
    /// Brings back the snoozed messages whose moment has come, unread, and says so once.
    fn wake_snoozed(&mut self) {
        let due = self.engine.session.snoozes_due(chrono::Local::now().timestamp());
        if due.is_empty() {
            return;
        }
        self.run("message.wake_snoozed", Value::Null);
        self.notice = Some(Notice::info(match due.as_slice() {
            [m] => crate::i18n::tr_args("snoozed-back-one", &[("subject", m.subject.as_str().into())]),
            list => crate::i18n::tr_args("snoozed-back", &[("count", list.len().into())]),
        }));
    }

    pub fn open_message(&mut self, id: &str) {
        let Some(folder) = self.engine.session.message(id).map(|m| m.folder_id.clone()) else { return };
        self.module = Module::Mail;
        self.drafts_for = None;
        // A message that the open folder lists (an "Alle accounts" one too) is shown there.
        let listed = self.engine.session.selected_folder.as_deref().is_some_and(|f| self.engine.session.members(f).contains(&folder));
        if !listed {
            self.run("folder.select", json!({ "id": folder }));
        }
        self.select_message(id);
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
        app.select_message(&id);
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
    helder::item_window(ctx, t, "message-window", &subject, vec2(820.0, 640.0), &mut open, |ui| {
        // Esc closes it, also when it is a window of its own and the main window does not see the key.
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.message_window = false;
        }
        egui::Frame::new().fill(t.reading).corner_radius(helder::CARD_RADIUS).inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            reading_pane(app, ui, t)
        });
    });
    if !open {
        app.message_window = false;
    }
}

fn reading_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(m) = app.engine.session.selected().cloned() else {
        ui.centered_and_justified(|ui| ui.label(RichText::new(tr("reading-select")).color(t.text_dim).size(15.0)));
        return;
    };
    if !app.engine.session.marked.is_empty() {
        // Several messages are selected: the pane says so, as Outlook does, and the ribbon's
        // actions take them all.
        let n = app.engine.session.marked.len() + 1;
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.label(RichText::new(crate::i18n::tr_args("selected-count", &[("count", n.into())])).size(18.0).strong().color(t.text));
            ui.add_space(8.0);
            ui.label(RichText::new(multi_hint()).color(t.text_dim));
        });
        return;
    }
    let in_junk = app.engine.session.folder(&m.folder_id).is_some_and(|f| f.kind == FolderKind::Junk);
    // "Snel antwoorden", as the new Outlook has it under a message: one click to a reply.
    if !in_junk && quick_reply(ui, t, m.from.display()) {
        app.open_compose(ComposeKind::Reply);
        return;
    }
    ui.label(RichText::new(&m.subject).size(20.0).strong().color(t.text));
    ui.add_space(10.0);
    importance::strip(ui, t, m.importance);
    if let Some(until) = app.engine.session.snoozed_until(&m, chrono::Local::now().timestamp()) {
        // Outlook's "Sluimeren": the moment the message comes back, and the way back now.
        let mut back = false;
        egui::Frame::new().fill(t.checked).corner_radius(4).inner_margin(8).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(crate::i18n::tr_args("snoozed-until", &[("when", compose::when_text(until).into())])).color(t.text));
                back = ui.small_button(message_menu::unsnooze()).on_hover_text(tr("reading-unsnooze-hint")).clicked();
            });
        });
        ui.add_space(8.0);
        if back {
            app.run("message.unsnooze", Value::Null);
        }
    }
    if m.flags.flagged {
        // The follow-up strip, as Outlook puts it over a flagged message: the due day, red once
        // it has passed, and the way to mark it done.
        let today = chrono::Local::now().date_naive();
        let due = app.engine.session.follow_up(&m.id).and_then(merel_engine::followup::due_day);
        let (text, late) = match due {
            Some(d) if d < today => (crate::i18n::tr_args("follow-up-overdue", &[("day", dates::day_name(d).into())]), true),
            Some(d) if d == today => (tr("follow-up-today"), false),
            Some(d) => (format!("⚑ {}", crate::i18n::tr_args("follow-up-before", &[("day", dates::day_name(d).into())])), false),
            None => (tr("follow-up-flagged"), false),
        };
        let mut done = false;
        egui::Frame::new().fill(if late { t.flag.gamma_multiply(0.15) } else { t.checked }).corner_radius(4).inner_margin(8).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(text).color(if late { t.flag } else { t.text }));
                done = ui.small_button(tr("reading-flag-done")).on_hover_text(tr("reading-flag-done-hint")).clicked();
            });
        });
        ui.add_space(8.0);
        if done {
            app.run("message.flag", Value::Null);
        }
    }
    if in_junk {
        // A message in the Junk folder: a warning, the links, attachments and pictures switched
        // off (as Outlook does), and the way back for a sender who is not junk.
        let mut not_junk = false;
        egui::Frame::new().fill(t.flag.gamma_multiply(0.15)).corner_radius(4).inner_margin(8).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(junk_notice()).color(t.text));
                not_junk = ui.small_button(tr("junk-not-junk")).on_hover_text(tr("junk-not-junk-hint")).clicked();
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
                ui.label(RichText::new(ignored_notice()).color(t.text));
                stop = ui.small_button(tr("ignore-stop")).on_hover_text(tr("ignore-stop-hint")).clicked();
            });
        });
        ui.add_space(8.0);
        if stop {
            app.run("conversation.unignore", json!({ "id": m.id }));
            return;
        }
    }
    let mut reply: Option<ComposeKind> = None;
    let mut sender = false;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        helder::avatar(ui.painter(), rect.center(), 20.0, m.from.display(), m.from.email());
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(m.from.display()).strong().color(t.text));
                if m.from.display() != m.from.email() {
                    ui.label(RichText::new(format!("<{}>", m.from.email())).color(t.text_dim));
                }
            });
            let to: Vec<&str> = m.to.iter().map(|a| a.display()).collect();
            ui.label(
                RichText::new(format!("{} · {}", crate::i18n::tr_args("to-line", &[("to", to.join("; ").into())]), dates::long(m.date)))
                    .size(12.0)
                    .color(t.text_dim),
            );
        });
        // The answers at hand in the message's own header, and the sender's contact card.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            helder::quiet(ui, t, |ui| {
                let known = app.engine.session.contact_with_email(m.from.email()).is_some();
                let (label, hint, icon) = if known {
                    (tr("contact-open"), tr("contact-open-hint"), helder::Icon::User)
                } else {
                    (tr("contact-add"), tr("contact-add-hint"), helder::Icon::UserPlus)
                };
                sender = header_button(ui, t, icon, &label).on_hover_text(hint).clicked();
                if !in_junk {
                    for (icon, label, kind) in [
                        (helder::Icon::Forward, &header_forward(), ComposeKind::Forward),
                        (helder::Icon::ReplyAll, &header_reply_all(), ComposeKind::ReplyAll),
                        (helder::Icon::Reply, &header_reply(), ComposeKind::Reply),
                    ] {
                        if header_button(ui, t, icon, label).on_hover_text(label).clicked() {
                            reply = Some(kind);
                        }
                    }
                }
            });
        });
    });
    if sender {
        app.open_or_add_sender(&m.from);
    }
    if let Some(kind) = reply {
        app.open_compose(kind);
        return;
    }
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
                ui.label(RichText::new(phishing_title()).strong().color(t.text));
                for w in &warnings {
                    ui.label(RichText::new(format!("• {}", w.text())).size(12.0).color(t.text));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tr("phishing-advice")).size(12.0).color(t.text_dim));
                    block = ui.small_button(tr("block-sender")).on_hover_text(tr("block-sender-hint")).clicked();
                });
            },
        );
        ui.add_space(8.0);
        if block {
            app.run("junk.block", json!({ "id": message_id }));
            return;
        }
    }
    if let Some(e) = body.as_ref().and_then(|b| b.encrypted.as_ref()) {
        signature::encryption_strip(ui, t, e);
    }
    if let Some(v) = body.as_ref().and_then(|b| b.pgp_signature.as_ref()) {
        signature::pgp_strip(ui, t, v, m.from.email());
    }
    if let Some(sig) = body.as_ref().and_then(|b| b.signature.as_ref())
        && signature::strip(ui, t, sig, m.from.email(), !in_junk)
    {
        match app.engine.trust_certificate(&message_id) {
            Ok(()) => app.notice = Some(Notice::success(tr("certificate-trusted"))),
            Err(e) => app.notice = Some(Notice::error(e)),
        }
    }
    // A list's mail offers the way off it, as Outlook does; not in the Junk folder, where the
    // header is not to be trusted.
    // A meeting invitation, as Outlook shows one over the message.
    if !in_junk && let Some(inv) = body.as_ref().and_then(|b| invitation::from_attachments(&b.attachments)) {
        let known = match &inv.answer {
            Some(answer) => app.has_answer(answer),
            None => inv.appointment.as_ref().is_some_and(|a| app.has_appointment_like(a)),
        };
        if let Some(a) = invitation::strip(ui, t, &inv, known) {
            app.add_invitation(&a);
        }
        ui.add_space(8.0);
    }
    if !in_junk && let Some(targets) = body.as_ref().map(|b| b.unsubscribe.as_slice()).filter(|u| !u.is_empty()) {
        let mut ask = false;
        egui::Frame::new().fill(t.checked).corner_radius(4).inner_margin(8).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(unsubscribe::strip()).color(t.text));
                ask = ui.small_button(tr("unsubscribe")).on_hover_text(tr("unsubscribe-hint")).clicked();
            });
        });
        ui.add_space(8.0);
        if ask {
            let targets = targets.to_vec();
            app.ask_unsubscribe(&message_id, m.from.display(), &targets);
        }
    }
    // The sender's request for a read receipt, as Outlook asks it. Never answered by itself, and
    // not in the Junk folder, where the address is not to be trusted.
    if !in_junk
        && let Some(to) = body.as_ref().and_then(|b| b.receipt_to.clone())
        && receipt::strip(ui, t, &to, app.receipts_sent.contains(&message_id))
    {
        app.send_read_receipt(&message_id);
    }
    let rich = body.as_ref().and_then(|b| b.rich.as_ref());
    let blocked = rich.map(|d| d.remote_images).unwrap_or(0);
    // A sender the person put on the safe senders list gets their pictures without asking, as
    // Outlook does for its Safe Senders; contacts do not count, this list is chosen on purpose.
    let safe_sender = app.engine.session.junk.is_safe(m.from.email());
    if blocked > 0
        && !in_junk
        && safe_sender
        && !app.remote.allowed(&message_id)
        && let Some(doc) = rich
    {
        app.remote.allow(ui.ctx(), &message_id, doc);
    }
    let allowed = app.remote.allowed(&message_id);
    // Privacy: nothing in a message runs or loads; pictures on the internet stay blocked until
    // the person asks for them, for this one message.
    let notice = match (rich, body.as_ref().is_some_and(|b| b.from_html)) {
        (Some(_), _) if blocked > 0 && allowed && safe_sender => Some(safe_images_notice()),
        (Some(_), _) if blocked > 0 && allowed => Some(tr("images-loaded-on-request")),
        (Some(_), _) if blocked > 0 => Some(tr("images-blocked")),
        (Some(_), _) => None,
        (None, true) => Some(tr("images-shown-as-text")),
        (None, false) => None,
    };
    if let Some(notice) = notice {
        let mut load = false;
        let mut always = false;
        egui::Frame::NONE.fill(t.checked).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(notice).size(12.0).color(t.text));
                if blocked > 0 && !allowed && !in_junk {
                    load = ui.small_button(tr("images-load")).on_hover_text(tr("images-load-hint")).clicked();
                    always = ui.small_button(always_from_sender()).on_hover_text(tr("images-always-hint")).clicked();
                }
            });
        });
        if always {
            app.run("junk.trust", serde_json::json!({ "id": message_id }));
        }
        if (load || always)
            && let Some(doc) = rich
        {
            app.remote.allow(ui.ctx(), &message_id, doc);
        }
        ui.add_space(8.0);
    }
    // The calendar part of an invitation or an answer is the strip above, not a file of its own,
    // as Outlook shows it; only a named .ics stays a tile.
    let files: Vec<&merel_mime::Attachment> = body
        .as_ref()
        .map(|b| b.attachments.iter().filter(|a| !(invitation::is_calendar(a) && !a.name.to_ascii_lowercase().ends_with(".ics"))).collect())
        .unwrap_or_default();
    if let Some(b) = body.as_ref().filter(|_| !files.is_empty()) {
        let mut save = None;
        let mut save_all = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for a in files.iter().copied() {
                let size = size_text(a.size);
                let label = format!("📎 {}  ({size})", a.name);
                let r = helder::file_tile(ui, t, &a.name, &size, &label, !in_junk);
                if in_junk {
                    r.on_hover_text(tr("attachment-junk-hint"));
                } else if r.on_hover_text(tr("attachment-save-hint")).clicked() {
                    save = Some(a.clone());
                }
            }
            // Several attachments: one click saves them all, as Outlook's "Alle bijlagen opslaan".
            if !in_junk && files.len() > 1 {
                helder::quiet(ui, t, |ui| {
                    if helder::tool_button(ui, t, helder::Icon::Download, &attachments::save_all_label())
                        .on_hover_text(tr("attachments-save-all-hint"))
                        .clicked()
                    {
                        save_all = true;
                    }
                });
            }
        });
        if let Some(a) = save {
            app.save_attachment(&a);
        }
        if save_all {
            app.save_attachments(&files.iter().map(|a| (*a).clone()).collect::<Vec<_>>());
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
        (None, true, _) => tr("message-fetching"),
        (None, false, true) => m.preview.clone(),
        (None, false, false) => String::new(),
    };
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(text).color(if body.is_some() || sample { t.text } else { t.text_dim }).size(14.0)).wrap());
    });
}

/// The answer buttons in the reading pane's header; named apart from the ribbon's own.
pub(crate) fn header_reply() -> String {
    tr("header-reply")
}
pub(crate) fn header_reply_all() -> String {
    tr("header-reply-all")
}
pub(crate) fn header_forward() -> String {
    tr("header-forward")
}
/// The bar under a message that starts a reply.
pub(crate) fn quick_reply_label() -> String {
    tr("quick-reply")
}

/// A quiet square icon button in the reading pane's header.
fn header_button(ui: &mut Ui, t: &Tokens, icon: helder::Icon, label: &str) -> egui::Response {
    let r = ui.add(
        egui::Button::image(icon.image(helder::ICON, t.text))
            .frame_when_inactive(false)
            .corner_radius(helder::CONTROL_RADIUS)
            .min_size(vec2(34.0, 34.0)),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    r
}

/// The "Snel antwoorden aan …" bar at the foot of the reading pane. Whether it was clicked.
fn quick_reply(ui: &mut Ui, t: &Tokens, to: &str) -> bool {
    let first = to.split_whitespace().next().unwrap_or(to);
    egui::Panel::bottom("quick-reply")
        .show_separator_line(false)
        .frame(egui::Frame::NONE.inner_margin(Margin { left: 0, right: 0, top: 10, bottom: 0 }))
        .show(ui, |ui| {
            let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, quick_reply_label()));
            let p = ui.painter();
            let fill = if r.hovered() { t.hover } else { t.card };
            p.rect(rect, CornerRadius::same(helder::CONTROL_RADIUS), fill, Stroke::new(1.0, t.control_border), egui::StrokeKind::Inside);
            let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0));
            helder::Icon::Reply.image(16.0, t.text_dim).paint_at(ui, icon);
            let p = ui.painter();
            p.text(
                egui::pos2(rect.left() + 38.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                crate::i18n::tr_args("quick-reply-to", &[("name", first.into())]),
                egui::FontId::proportional(14.0),
                t.text_dim,
            );
            r.on_hover_text(tr("quick-reply-hint")).clicked()
        })
        .inner
}

/// The first line of the strip above a message that shows signs of phishing.
pub(crate) fn phishing_title() -> String {
    tr("phishing-title")
}
/// The strip above a message of an ignored conversation.
pub(crate) fn ignored_notice() -> String {
    tr("ignored-notice")
}
/// The strip above a message in the Junk folder.
pub(crate) fn junk_notice() -> String {
    tr("junk-notice")
}

/// What points to phishing in `m`, from its sender (against the contacts and the person's own
/// addresses) and, once fetched, its body: the server's authentication results, the links and
/// the attachments.
fn message_warnings(app: &MailApp, m: &merel_model::MessageHeader, body: Option<&merel_engine::Body>) -> Vec<Warning> {
    let s = &app.engine.session;
    let own: Vec<&str> = s.accounts.iter().map(|a| a.address.email()).collect();
    let mut out = merel_model::sender_warnings(&m.from, &s.contacts, &own);
    if let Some(b) = body {
        out.extend(merel_model::authentication_warning(&b.authentication));
        out.extend(merel_model::link_warnings(&b.rich.as_ref().map(|d| d.links()).unwrap_or_default()));
        out.extend(merel_model::attachment_warnings(b.attachments.iter().map(|a| a.name.as_str())));
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

/// The name of a list filter in the current language.
fn filter_name(filter: merel_engine::view::Filter) -> String {
    use merel_engine::view::Filter;
    tr(match filter {
        Filter::All => "filter-all",
        Filter::Unread => "filter-unread",
        Filter::Flagged => "filter-flagged",
        Filter::Attachments => "filter-attachments",
        Filter::Snoozed => "filter-snoozed",
    })
}

fn status_bar(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::bottom("status").show_separator_line(false).frame(egui::Frame::NONE.fill(t.window).inner_margin(Margin::symmetric(12, 4))).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                let list = app.engine.session.visible_messages();
                let unread = list.iter().filter(|m| !m.flags.seen).count();
                ui.label(
                    RichText::new(crate::i18n::tr_args("status-items", &[("items", list.len().into()), ("unread", unread.into())]))
                        .size(12.0)
                        .color(t.text_dim),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (txt, col) = if app.engine.session.is_sample() {
                        (tr("status-sample"), t.text_dim)
                    } else {
                        let statuses = app.engine.session.status.values();
                        let mut worst = (tr("account-online"), t.text_dim);
                        for s in statuses {
                            match s {
                                AccountStatus::Error(e) => worst = (e.clone(), t.flag),
                                AccountStatus::Connecting if worst.1 != t.flag => worst = (tr("account-connecting"), t.text_dim),
                                _ => {}
                            }
                        }
                        if app.engine.session.has_pending() && worst.1 != t.flag {
                            worst.0 = tr("status-busy");
                        }
                        let queued: usize = app.engine.session.queued.values().sum();
                        match queued {
                            0 => {}
                            n => worst = (crate::i18n::tr_args("status-queued", &[("count", n.into())]), t.flag),
                        }
                        worst
                    };
                    ui.label(RichText::new(txt).size(12.0).color(col));
                });
            });
        },
    );
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
    fn with_a_native_menu_the_tabs_live_in_the_menu_bar_and_its_clicks_run_them() {
        let mut h = harness();
        h.state_mut().native_menu = true;
        h.run();
        assert!(h.query_by_label("Verzenden/ontvangen").is_none(), "no tabs in the window");
        h.get_by_label("Nieuwe e-mail");
        let titles: Vec<&str> = h.state().menu_bar.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(titles, ["Bestand", "Bericht", "Map", "Weergeven", "Help"]);
        let bericht = &h.state().menu_bar[1];
        assert!(bericht.entries.iter().any(|e| matches!(e, menubar::Entry::Item { label, enabled: false, .. } if label == "Beantwoorden")));
        // A choice in a submenu, the way the menu bar hands it over.
        let view = h.state().menu_bar.iter().position(|m| m.title == "Weergeven").unwrap();
        h.state_mut().click_menu(view, vec!["Thema".into(), "Donker".into()]);
        h.run();
        assert_eq!(h.state().view.theme, view::ThemeChoice::Dark);
        h.state_mut().click_menu(view, vec!["Agenda".into()]);
        h.run();
        assert_eq!(h.state().module, Module::Calendar);
        assert_eq!(h.state().menu_bar[1].title, "Agenda");
        h.state_mut().click_menu(0, vec!["Nieuwe e-mail".into()]);
        h.run();
        assert!(h.state().compose.is_some(), "Bestand > Nieuwe e-mail opens a new message");
        h.state_mut().click_menu(0, vec!["Accounts…".into()]);
        h.run();
        assert_eq!(h.state().tab, Tab::File);
    }

    #[test]
    fn the_welcome_walks_through_its_steps_and_comes_only_once() {
        let mut h = harness();
        h.run();
        assert!(h.state().welcome.is_none(), "the sample has no settings folder, so no welcome");
        h.state_mut().welcome = Some(welcome::Step::Hello);
        h.run();
        assert!(h.query_all_by_label(&welcome::window_label()).count() >= 1);
        h.get_by_label("Stap 1 van 4");
        h.get_by_label(&welcome::next_label()).click();
        h.run();
        h.get_by_label(&welcome::add_account());
        h.get_by_label(&welcome::back_label()).click();
        h.run();
        h.get_by_label("Stap 1 van 4");
        h.get_by_label(&welcome::next_label()).click();
        h.run();
        h.get_by_label(&welcome::next_label()).click();
        h.run();
        assert_eq!(h.state().view.update_check, Some(false), "shown is chosen: no until said otherwise");
        h.get_by_label(&welcome::updates_on()).click();
        h.run();
        assert_eq!(h.state().view.update_check, Some(true));
        h.get_by_label(&welcome::next_label()).click();
        h.run();
        h.get_by_label("Donker").click();
        h.run();
        assert_eq!(h.state().view.theme, view::ThemeChoice::Dark);
        h.get_by_label(&welcome::done()).click();
        h.run();
        assert!(h.state().welcome.is_none());
        assert!(h.state().view.welcomed);
        assert!(!welcome::due(h.state()));
    }

    #[test]
    fn adding_an_account_from_the_welcome_opens_the_account_form() {
        let mut h = harness();
        h.state_mut().welcome = Some(welcome::Step::Account);
        h.run();
        h.get_by_label(&welcome::add_account()).click();
        h.run();
        assert!(h.state().welcome.is_none());
        assert!(h.state().view.welcomed);
        assert_eq!(h.state().view.update_check, Some(false), "skipped means no daily check");
        assert_eq!(h.state().tab, Tab::File);
    }

    #[test]
    fn a_newer_release_is_offered_once_and_a_skipped_one_not_again() {
        use updates::{Outcome, Release};
        let mut h = harness();
        h.run();
        h.get_by_label("Help").click();
        h.run();
        h.get_by_label(&updates::check());
        let release = Release { version: "9.0.0".into(), notes: "Nieuw: van alles.".into() };
        h.state_mut().finish_update_check(Outcome::Newer(release.clone()));
        h.run();
        assert!(h.query_all_by_label(&updates::window_label()).count() >= 1);
        h.get_by_label("Merel 9.0.0 is beschikbaar.");
        h.get_by_label("Nieuw: van alles.");
        h.get_by_label(&updates::download());
        h.get_by_label(&updates::skip()).click();
        h.run();
        assert!(h.state().updates.offer.is_none());
        assert_eq!(h.state().view.skipped_version.as_deref(), Some("9.0.0"));
        // The daily check keeps quiet about the skipped release, but asking still shows it.
        h.state_mut().finish_update_check(Outcome::Newer(release.clone()));
        assert!(h.state().updates.offer.is_none());
        h.state_mut().finish_update_check(Outcome::Newer(Release { version: "9.1.0".into(), notes: String::new() }));
        assert!(h.state().updates.offer.is_some());
        h.run();
        h.get_by_label(&updates::later()).click();
        h.run();
        assert!(h.state().updates.offer.is_none());
        // Only an answer to the person's own question is spoken.
        h.state_mut().finish_update_check(Outcome::UpToDate);
        assert!(h.state().notice.is_none());
        h.state_mut().finish_update_check(Outcome::Failed("offline".into()));
        assert!(h.state().notice.is_none());
    }

    #[test]
    fn the_update_wizard_shows_each_stage_and_never_walks_away_from_an_install() {
        use install::{Job, Restart, Stage};
        let mut h = harness();
        h.run();
        h.state_mut().updates.job = Some(Job::at("9.0.0", Stage::Downloading { done: 12_000_000, total: Some(38_000_000) }));
        h.run();
        h.get_by_label("Merel 9.0.0 downloaden");
        h.get_by_label("12 van 38 MB");
        h.get_by_label("Stap 2 van 4");
        h.get_by_label(&updates::cancel()).click();
        h.run();
        assert!(h.state().updates.job.is_none(), "cancelling ends the download");

        h.state_mut().updates.job = Some(Job::at("9.0.0", Stage::Ready(Restart::Relaunch("/Applications/Merel.app".into()))));
        h.run();
        h.get_by_label("Merel 9.0.0 is geïnstalleerd");
        h.get_by_label(&updates::restart_label());
        h.get_by_label(&updates::later()).click();
        h.run();
        h.run();
        assert!(h.state().updates.job.is_none());

        h.state_mut().updates.job = Some(Job::at("9.0.0", Stage::Installing));
        h.run();
        h.get_by_label("Merel 9.0.0 installeren");
        h.get_by_label(&helder::close_label()).click();
        h.run();
        assert!(h.state().updates.job.is_some(), "an install in progress is not closed");

        h.state_mut().updates.job = Some(Job::at("9.0.0", Stage::Ready(Restart::Relaunch("/x".into()))));
        h.run();
        h.run();
        h.get_by_label(&updates::later()).click();
        h.run();
        assert!(h.state().updates.job.is_none(), "once installed, Later closes it");
        h.state_mut().updates.job =
            Some(Job::at("9.0.0", Stage::Failed("De handtekening van deze versie klopt niet. Er is niets geïnstalleerd.".into())));
        h.run();
        h.get_by_label("Bijwerken is niet gelukt");
        h.get_by_label(&updates::page_label());
        assert!(h.query_by_label_contains("De handtekening van deze versie klopt niet").is_some());
    }

    #[test]
    fn the_daily_update_check_is_off_until_chosen_and_switched_in_over_merel() {
        let mut h = harness();
        h.run();
        assert_eq!(h.state().view.update_check, None);
        assert!(!h.state().updates.running(), "nothing goes out before the person chose");
        h.get_by_label("Help").click();
        h.run();
        h.get_by_label("Over Merel").click();
        h.run();
        h.get_by_label(&updates::auto()).click();
        h.run();
        assert_eq!(h.state().view.update_check, Some(true));
        assert!(h.state().updates.running(), "switching it on asks at once");
    }

    #[test]
    fn a_notice_shows_at_the_top_with_its_kind_and_goes_by_itself_unless_it_is_an_error() {
        let mut h = harness();
        h.run();
        h.state_mut().notice = Some(Notice::success("Contact opgeslagen in Downloads: Ravi Kumar.vcf").revealing("/tmp/Ravi Kumar.vcf"));
        h.run();
        assert!(h.get_by_label(&notice::close_label()).rect().top() < 220.0, "just under the ribbon, where the person looks");
        h.get_by_label(&notice::reveal_label());
        for _ in 0..30 {
            h.step();
        }
        assert!(h.state().notice.is_none(), "good news goes by itself");

        h.state_mut().notice = Some(Notice::error("Opslaan is mislukt."));
        for _ in 0..60 {
            h.step();
        }
        assert!(h.state().notice.is_some(), "an error waits to be read");
        assert!(h.query_by_label(&notice::reveal_label()).is_none(), "nothing to show in a folder");
        h.get_by_label(&notice::close_label()).click();
        h.run();
        assert!(h.state().notice.is_none());
    }

    #[test]
    fn a_command_that_works_clears_an_earlier_error_but_not_good_news() {
        let mut h = harness();
        h.run();
        h.state_mut().select_message("m1");
        h.state_mut().notice = Some(Notice::error("Dit ging mis."));
        h.state_mut().run("message.mark_read", Value::Null);
        assert!(h.state().notice.is_none());
        h.state_mut().notice = Some(Notice::success("Bericht verzonden."));
        h.state_mut().run("message.mark_read", Value::Null);
        assert_eq!(h.state().notice.as_deref(), Some("Bericht verzonden."));
    }

    #[test]
    fn the_server_can_be_asked_to_search_a_connected_folder() {
        use merel_engine::{Event, RemoteFolder, RemoteHeader};
        let mut engine = Engine::sample();
        let account = {
            let mut f = crate::backstage::AccountForm::default();
            f.email = "alain@voorbeeld.nl".into();
            f.password = "x".into();
            f.suggest();
            f.account().expect("account")
        };
        let id = account.id.clone();
        engine.add_account(account, merel_secrets::Secret::new("x")).expect("add");
        engine.session.apply(Event::Connected {
            account_id: id.clone(),
            folders: vec![RemoteFolder {
                path: "INBOX".into(),
                name: "Inbox".into(),
                kind: merel_model::FolderKind::Inbox,
                selectable: true,
                parent: None,
            }],
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
            header: merel_mime::parse_header(b"From: boekhouding@voorbeeld.nl\r\nSubject: Jaaroverzicht\r\n\r\n"),
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
    fn doorsturen_als_bijlage_attaches_the_message_itself_as_an_eml_file() {
        use egui::{Key, Modifiers};
        let mut h = harness();
        h.run();
        let id = h.state().engine.session.visible_messages().first().map(|m| m.id.clone()).expect("sample mail");
        h.state_mut().select_message(&id);
        h.run();
        let m = h.state().engine.session.selected().cloned().expect("a message is selected");
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::F);
        h.run();
        let c = h.state().compose.as_ref().expect("a new message opens");
        assert!(c.subject.starts_with("Fw:"), "{}", c.subject);
        assert!(!c.body.contains("schreef"), "nothing is quoted, the message travels whole: {}", c.body);
        assert_eq!(c.attachments.len(), 1);
        let a = &c.attachments[0];
        assert_eq!((a.name.as_str(), a.mime.as_str()), (crate::eml::file_name(&m.subject).as_str(), "message/rfc822"));
        let source = String::from_utf8_lossy(&a.bytes);
        assert!(source.contains(&format!("Subject: {}", m.subject)) && source.contains(m.from.email()), "{source}");
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
        h.get_by_label(&search_clear()).click();
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

    /// An IMAP account `a1` with one unread chat message from Sanne in its inbox.
    fn with_a_chat(app: &mut MailApp) {
        use merel_model::{Account, Address, Auth, ChatTag, Flags, Folder, MessageHeader, Protocol, Server, Tls};
        let s = &mut app.engine.session;
        let me = Address::new(Some("Alain Rees"), "alain@voorbeeld.nl").unwrap();
        s.accounts.push(Account {
            id: "a1".into(),
            name: "Alain Rees".into(),
            address: me.clone(),
            username: "alain@voorbeeld.nl".into(),
            protocol: Protocol::ImapSmtp,
            auth: Auth::Password,
            incoming: Some(Server { host: "127.0.0.1".into(), port: 1, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        });
        s.folders.push(Folder {
            id: "a1/INBOX".into(),
            account_id: "a1".into(),
            name: "Postvak IN".into(),
            path: "INBOX".into(),
            kind: FolderKind::Inbox,
            unread: 1,
            parent: None,
        });
        s.messages.push(MessageHeader {
            id: "a1/INBOX#1".into(),
            folder_id: "a1/INBOX".into(),
            remote_id: "1".into(),
            from: Address::new(Some("Sanne de Vries"), "sanne@example.org").unwrap(),
            to: vec![me],
            subject: "Chatbericht".into(),
            date: 1_792_000_000,
            preview: String::new(),
            flags: Flags { seen: false, flagged: false, answered: false },
            has_attachments: false,
            message_id: Some("<c1@example.org>".into()),
            categories: Vec::new(),
            importance: merel_model::Importance::Normal,
            chat: Some(ChatTag::default()),
            sealed: Default::default(),
        });
        s.bodies.insert("a1/INBOX#1".into(), merel_engine::Body { text: "Zie ik je om tien uur?".into(), ..Default::default() });
    }

    #[test]
    fn the_chat_module_lists_a_chat_opens_it_and_sends_only_encrypted() {
        let mut h = harness();
        with_a_chat(h.state_mut());
        h.state_mut().module = Module::Chat;
        h.run();
        h.get_by_label("Chat staat uit voor alain@voorbeeld.nl.");
        h.get_by_label("Sanne de Vries, 1 ongelezen").click();
        h.run();
        let id = merel_engine::chat::person_chat_id("a1", "sanne@example.org");
        assert_eq!(h.state().chat.selected.as_deref(), Some(id.as_str()));
        assert_eq!(h.state().engine.session.chat(&id).unwrap().unread, 0, "opening reads it");
        h.get_by_label("Zie ik je om tien uur?");
        h.state_mut().chat.input = "Ja, tot dan".into();
        h.run();
        h.get_by_label("Verzenden").click();
        h.run();
        assert_eq!(h.state().chat.error.as_deref(), Some("Zet eerst chat aan voor dit account."));
        assert_eq!(h.state().chat.input, "Ja, tot dan", "what was typed stays");
        assert!(h.state().engine.session.outbox.is_empty(), "nothing went out");

        h.get_by_label("Nieuwe chat").click();
        h.run();
        assert_eq!(h.state().chat.new.as_ref().map(|f| f.account_id.as_str()), Some("a1"));
        h.get_by_label("Annuleren").click();
        h.run();
        assert!(h.state().chat.new.is_none());
    }

    #[test]
    fn the_chat_start_page_shows_an_invitation_link_and_takes_one() {
        let mut h = harness();
        with_a_chat(h.state_mut());
        h.state_mut().module = Module::Chat;
        h.state_mut().engine.enable_chat("a1").unwrap();
        h.run();
        h.get_by_label("Uitnodigingslink").click();
        h.run();
        let (account, link) = h.state().chat.invite.clone().unwrap();
        assert_eq!(account, "a1");
        assert!(link.starts_with("https://i.delta.chat/#"), "{link}");
        h.get_by_label("QR-code van de uitnodigingslink");
        h.get_by_label("Link kopiëren").click();
        h.run();
        h.get_by_label("Gekopieerd");
        h.get_by_label("Per e-mail sturen").click();
        h.run();
        assert!(h.state().compose.as_ref().is_some_and(|c| c.body.contains(&link) && c.account_id == "a1"));
        h.state_mut().compose = None;
        h.run();

        h.state_mut().chat.accept = "geen link".into();
        h.run();
        h.get_by_label("Aannemen").click();
        h.run();
        assert!(matches!(&h.state().chat.accept_note, Some(Err(e)) if e.contains("geen uitnodiging")));
        h.state_mut().chat.accept = "https://i.delta.chat/#1234567890ABCDEF1234567890ABCDEF12345678&i=abc&s=def&a=bob%40example.org&n=Bob".into();
        h.run();
        h.get_by_label("Aannemen").click();
        // The first step waits to go out, so the window keeps waking.
        h.run_steps(3);
        h.get_by_label("Uitnodiging aangenomen. Zodra Bob antwoordt, staat de chat klaar.");
        h.get_by_label("Wacht op antwoord van Bob (bob@example.org)");
    }

    #[test]
    fn chat_messages_stay_out_of_the_mail_list() {
        let mut h = harness();
        with_a_chat(h.state_mut());
        h.state_mut().engine.session.selected_folder = Some("a1/INBOX".into());
        h.run();
        assert!(h.query_by_label("Zie ik je om tien uur?").is_none());
        assert!(h.state().engine.session.visible_messages().is_empty());
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
            e.first = "Noor".into();
            e.last = "Bakker".into();
            e.emails = vec!["noor@example.org".into()];
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
    fn gelezen_markeren_can_wait_a_few_seconds_or_be_left_to_the_person() {
        let mut h = harness();
        let id = h.state().engine.session.messages.iter().find(|m| !m.flags.seen).map(|m| m.id.clone()).expect("an unread sample message");
        h.state_mut().view.mark_read = view::MarkRead::Never;
        h.state_mut().select_message(&id);
        h.run();
        assert!(!h.state().engine.session.message(&id).unwrap().flags.seen, "not by itself");
        h.state_mut().view.mark_read = view::MarkRead::AfterSeconds;
        h.state_mut().select_message(&id);
        h.run();
        assert!(!h.state().engine.session.message(&id).unwrap().flags.seen, "not yet");
        h.state_mut().read_timer = Some((id.clone(), std::time::Instant::now() - std::time::Duration::from_secs(6)));
        h.run();
        assert!(h.state().engine.session.message(&id).unwrap().flags.seen, "read once it has been open long enough");
        assert!(h.state().read_timer.is_none());
        h.get_by_label("Beeld").click();
        h.run();
        h.get_by_label(&view::mark_read_menu()).click();
        h.run();
        h.get_all_by_label("Meteen").next().expect("the menu item").click();
        h.run();
        assert_eq!(h.state().view.mark_read, view::MarkRead::Now);
        h.get_by_label("Donker");
    }

    #[test]
    fn ga_naar_datum_moves_the_calendar_to_a_typed_day() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        h.get_all_by_label(&calendar::goto()).next().expect("the ribbon button").click();
        h.run();
        assert!(h.state().calendar.goto.is_some());
        *h.state_mut().calendar.goto.as_mut().expect("the window") = "31-02-2027".into();
        h.run();
        h.get_by_label(&calendar::goto_hint());
        h.get_by_label("Ga").click();
        h.run();
        assert!(h.state().calendar.goto.is_some(), "a day that does not exist keeps the window open");
        *h.state_mut().calendar.goto.as_mut().expect("the window") = "1-3-2027".into();
        h.run();
        h.get_by_label("Ga").click();
        h.run();
        assert_eq!(h.state().calendar.focus, chrono::NaiveDate::from_ymd_opt(2027, 3, 1).unwrap());
        assert!(h.state().calendar.goto.is_none());
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::G);
        h.run();
        assert_eq!(h.state().calendar.goto.as_deref(), Some("01-03-2027"), "Ctrl+G opens it on the shown day");
        h.get_by_label("Annuleren").click();
        h.run();
        assert!(h.state().calendar.goto.is_none());
    }

    #[test]
    fn werkuren_on_the_beeld_tab_switches_the_shading_of_the_hours_outside_the_working_day() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        assert!(h.state().view.work_hours, "on by default, as in Outlook");
        h.get_by_label("Beeld").click();
        h.run();
        h.get_by_label(&calendar::work_hours()).click();
        h.run();
        assert!(!h.state().view.work_hours);
        h.get_by_label("Start").click();
        h.run();
        h.get_by_label("Week").click();
        h.run();
        h.get_by_label("Beeld").click();
        h.run();
        h.get_by_label(&calendar::work_hours()).click();
        h.run();
        assert!(h.state().view.work_hours, "and back on, with the week view drawing the shade");
    }

    #[test]
    fn weeknummers_on_the_beeld_tab_number_the_weeks_of_the_calendar() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        let week = calendar::week_label(chrono::Local::now().date_naive());
        assert!(h.query_by_label(&week).is_none(), "off by default, as in Outlook");
        h.get_by_label("Beeld").click();
        h.run();
        h.get_by_label("Weeknummers").click();
        h.run();
        assert!(h.state().view.week_numbers);
        assert!(h.query_all_by_label(&week).count() >= 1, "the month view numbers the week of today");
        h.get_by_label("Start").click();
        h.run();
        h.get_by_label("Week").click();
        h.run();
        h.get_by_label(&week);
        h.get_by_label("Dag").click();
        h.run();
        h.get_by_label(&week);
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
    fn doorsturen_in_the_appointment_window_opens_a_message_with_the_appointment_attached() {
        let mut h = harness();
        let a = h.state().engine.session.appointments.first().cloned().expect("a sample appointment");
        h.state_mut().open_appointment(&a.id, a.start);
        h.run();
        h.get_by_label(&meeting::forward_label()).click();
        h.run();
        assert!(h.state().calendar.editor.is_none(), "the appointment window closed");
        let c = h.state().compose.as_ref().expect("the compose window");
        assert_eq!(c.subject, format!("Doorgestuurd: {}", a.title));
        assert!(c.to.is_empty() && c.body.contains(&a.title) && c.body.contains("Wanneer:"), "{}", c.body);
        assert_eq!(
            c.attachments.iter().map(|x| (x.name.as_str(), x.mime.as_str())).collect::<Vec<_>>(),
            [(meeting::FORWARD_FILE, meeting::FORWARD_MIME)]
        );
        assert_eq!(c.account_id, h.state().engine.session.current_account().map(|x| x.id.clone()).unwrap_or_default());
    }

    #[test]
    fn the_appointment_notes_have_the_formatting_bar_and_keep_their_formatting() {
        let mut h = harness();
        let mut e = calendar::Editor::new_at(chrono::Local::now().naive_local());
        e.title = "Werkoverleg".into();
        e.notes = "Agenda\nbegroting".into();
        let id = e.notes_editor.id();
        h.state_mut().calendar.editor = Some(e);
        h.run();
        h.get_by_label(&tr("notes")).focus();
        h.run();
        // "Agenda" selected, as dragging over it would.
        let mut state = egui::TextEdit::load_state(&h.ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(6))));
        state.store(&h.ctx, id);
        h.run();
        h.get_by_label(&rich_edit::bold()).click();
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        let a = h.state().engine.session.appointments.iter().find(|a| a.title == "Werkoverleg").cloned().expect("saved");
        let rich = a.notes_rich.clone().expect("formatted");
        assert!(rich.style_of(0).bold && !rich.style_of(7).bold, "{:?}", rich.runs);
        let back = calendar::Editor::from_appointment(&a, None);
        assert!(back.notes_editor.rich.style_of(0).bold, "the window shows it again");
    }

    #[test]
    fn a_day_picked_from_the_calendar_fills_the_date_and_the_end_moves_along() {
        let mut h = harness();
        let mut e = calendar::Editor::new_at(chrono::Local::now().naive_local());
        e.start_date = "05-10-2026".into();
        e.end_date = "06-10-2026".into();
        h.state_mut().calendar.editor = Some(e);
        h.run();
        h.get_by_label(&calendar::start_picker()).click();
        h.run();
        h.get_by_label("oktober 2026");
        h.get_by_label("Volgende maand").click();
        h.run();
        h.get_by_label("november 2026");
        h.get_by_label("12 november 2026").click();
        h.run();
        let e = h.state().calendar.editor.as_ref().expect("still open");
        assert_eq!((e.start_date.as_str(), e.end_date.as_str()), ("12-11-2026", "13-11-2026"), "a day long, as before");
        assert!(h.query_by_label("Volgende maand").is_none(), "the month closes once a day is picked");
        h.get_by_label(&calendar::end_picker()).click();
        h.run();
        h.get_by_label("20 november 2026").click();
        h.run();
        assert_eq!(h.state().calendar.editor.as_ref().expect("open").end_date, "20-11-2026");
    }

    #[test]
    fn a_task_due_day_can_be_picked_from_the_calendar() {
        let mut h = harness();
        h.state_mut().module = Module::Tasks;
        h.run();
        h.get_by_label("Nieuwe taak").click();
        h.run();
        h.state_mut().tasks.editor.as_mut().expect("editor").due = "30-11-2026".into();
        h.run();
        h.get_by_label(&tasks::due_picker()).click();
        h.run();
        h.get_by_label("1 december 2026").click();
        h.run();
        assert_eq!(h.state().tasks.editor.as_ref().expect("editor").due, "01-12-2026");
    }

    #[test]
    fn contact_and_task_notes_have_the_formatting_bar_and_keep_their_formatting() {
        // Selects "Agenda" in the notes of the open form and makes it bold, as a drag and a
        // click on the bar would.
        fn bold_first_word(h: &mut egui_kittest::Harness<'static, MailApp>, id: egui::Id) {
            h.get_by_label(&tr("notes")).focus();
            h.run();
            let mut state = egui::TextEdit::load_state(&h.ctx, id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(6))));
            state.store(&h.ctx, id);
            h.run();
            h.get_by_label(&rich_edit::bold()).click();
            h.run();
            h.get_by_label("Opslaan").click();
            h.run();
        }
        let mut h = harness();
        h.state_mut().module = Module::People;
        h.state_mut().new_contact();
        if let Some(e) = h.state_mut().contacts.editor.as_mut() {
            e.first = "Noor".into();
            e.notes = "Agenda\nbelt liever".into();
        }
        let id = h.state().contacts.editor.as_ref().map(|e| e.notes_editor.id()).expect("form");
        h.run();
        bold_first_word(&mut h, id);
        let c = h.state().engine.session.contacts.iter().find(|c| c.name == "Noor").cloned().expect("saved");
        let rich = c.notes_rich.clone().expect("formatted");
        assert!(rich.style_of(0).bold && !rich.style_of(7).bold, "{:?}", rich.runs);
        assert!(contacts::Editor::from_contact(&c).notes_editor.rich.style_of(0).bold, "the form shows it again");

        h.state_mut().module = Module::Tasks;
        h.state_mut().new_task();
        if let Some(e) = h.state_mut().tasks.editor.as_mut() {
            e.title = "Overleg voorbereiden".into();
            e.notes = "Agenda\nbegroting".into();
        }
        let id = h.state().tasks.editor.as_ref().map(|e| e.notes_editor.id()).expect("form");
        h.run();
        bold_first_word(&mut h, id);
        let task = h.state().engine.session.tasks.iter().find(|t| t.title == "Overleg voorbereiden").cloned().expect("saved");
        assert!(task.notes_rich.as_ref().is_some_and(|r| r.style_of(0).bold), "{:?}", task.notes_rich);
        assert!(tasks::Editor::from_task(&task).notes_editor.rich.style_of(0).bold);
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
        h.get_by_label(&meeting::no_account());
        assert!(h.state().calendar.editor.is_some(), "the window stays open");
        assert!(h.state().engine.session.appointments.iter().all(|a| a.title != "Overleg met Sanne"), "nothing was saved");
        h.state_mut().calendar.editor.as_mut().expect("editor").attendees = "sanne@".into();
        h.run();
        h.get_by_label("Verzenden").click();
        h.run();
        h.get_by_label(&meeting::bad_addresses());
    }

    fn with_real_account(h: &mut egui_kittest::Harness<'static, MailApp>) {
        let s = &mut h.state_mut().engine.session;
        s.accounts.insert(
            0,
            merel_model::Account {
                id: "acc".into(),
                name: "MXroute".into(),
                address: merel_model::Address::new(Some("Alain Tester"), "alain@voorbeeld.nl").unwrap(),
                username: "alain@voorbeeld.nl".into(),
                protocol: merel_model::Protocol::ImapSmtp,
                auth: merel_model::Auth::Password,
                incoming: Some(merel_model::Server { host: "127.0.0.1".into(), port: 1, tls: merel_model::Tls::Implicit }),
                outgoing: None,
                dav: None,
                signature: String::new(),
            },
        );
        s.selected_folder = None;
    }

    #[test]
    fn invited_people_stay_as_chips_and_their_answers_show_in_the_meeting() {
        let mut h = harness();
        with_real_account(&mut h);
        let tomorrow = chrono::Local::now().naive_local() + chrono::TimeDelta::days(1);
        let mut e = calendar::Editor::new_at(tomorrow.date().and_hms_opt(12, 0, 0).unwrap());
        e.title = "Lunch".into();
        e.attendees = "Alain Tester <alain@werk.example.org>; ".into();
        h.state_mut().calendar.editor = Some(e);
        h.run();
        {
            let e = h.state().calendar.editor.as_ref().expect("editor");
            assert_eq!((e.invited.len(), e.attendees.as_str()), (1, ""), "the finished address became a chip");
        }
        h.get_by_label("Alain Tester verwijderen");
        assert!(h.query_by_label(&calendar::in_the_past()).is_none());
        h.get_by_label("Verzenden").click();
        h.run_steps(3);
        assert!(h.state().calendar.editor.is_none(), "sent and closed");
        let a = h.state().engine.session.appointments.iter().find(|a| a.title == "Lunch").cloned().expect("saved");
        assert!(!a.notes.contains("Genodigden"), "the people are not in the notes: {:?}", a.notes);
        let m = a.meeting.clone().expect("a meeting");
        assert_eq!((m.organizer.as_str(), m.attendees.len(), m.attendees[0].email.as_str()), ("alain@voorbeeld.nl", 1, "alain@werk.example.org"));
        assert_eq!(h.state().engine.session.outbox.len(), 1, "the invitation went to the outbox");

        // The acceptance comes in and is filed.
        let reply = format!(
            "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nUID:{}\r\nDTSTAMP:20261020T210000Z\r\nDTSTART:20261027T110000Z\r\nDTEND:20261027T120000Z\r\nSUMMARY:Lunch\r\nATTENDEE;CN=Alain Tester;PARTSTAT=ACCEPTED:mailto:alain@werk.example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            m.uid
        );
        let answer = merel_engine::meetings::parse_answer(&reply).expect("an answer");
        assert!(h.state_mut().engine.session.apply_answer(&answer, "alain@werk.example.org").is_some());
        h.state_mut().open_appointment(&a.id, a.start);
        h.run_steps(3);
        assert!(h.query_all_by_label_contains("1 geaccepteerd").count() >= 1, "the window says who answered");
        assert!(h.query_by_label("Opslaan").is_some(), "an unchanged meeting is only saved");
        // Taking the person off sends them a cancellation with the update.
        h.get_by_label("Alain Tester verwijderen").click();
        h.run_steps(3);
        h.get_by_label(&meeting::send_update()).click();
        h.run_steps(3);
        assert_eq!(h.state().engine.session.outbox.len(), 2, "the cancellation went out too");
        let a = h.state().engine.session.appointments.iter().find(|a| a.title == "Lunch").cloned().expect("saved");
        assert!(a.meeting.is_none(), "nobody is invited any more");
    }

    #[test]
    fn a_meeting_sent_by_an_older_version_shows_its_people_as_chips() {
        let mut h = harness();
        with_real_account(&mut h);
        let id = {
            let s = &mut h.state_mut().engine.session;
            let a = s.appointments.iter_mut().find(|a| a.title == "Lunch met Sanne").expect("sample");
            a.notes = "Genodigden: Alain Tester <alain@werk.example.org>".into();
            a.id.clone()
        };
        let start = h.state().engine.session.appointment(&id).map(|a| a.start).unwrap();
        h.state_mut().open_appointment(&id, start);
        h.run();
        let e = h.state().calendar.editor.as_ref().expect("editor");
        assert_eq!((e.invited.len(), e.notes.as_str(), e.legacy), (1, "", true));
        h.get_by_label("Alain Tester verwijderen");
        assert!(h.query_by_label("Opslaan").is_some(), "nothing changed, so nothing is sent again");
        h.get_by_label("Opslaan").click();
        h.run();
        let a = h.state().engine.session.appointment(&id).cloned().expect("saved");
        let m = a.meeting.expect("now a meeting");
        assert!(m.uid.ends_with("@mailcraft.voorbeeld.nl") && m.uid.contains(&format!("-{start}@")), "{}", m.uid);
        assert!(a.notes.is_empty() && h.state().engine.session.outbox.is_empty());
    }

    #[test]
    fn an_answer_in_the_mail_says_who_accepted() {
        let mut h = harness();
        {
            let s = &mut h.state_mut().engine.session;
            let ics = "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nUID:x1@mailcraft.example.org\r\nDTSTART:20261027T110000Z\r\nDTEND:20261027T120000Z\r\nSUMMARY:Lunch\r\nATTENDEE;CN=Sanne;PARTSTAT=ACCEPTED:mailto:sanne@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
            let part = merel_mime::Attachment {
                name: "invite.ics".into(),
                mime: "text/calendar".into(),
                size: ics.len(),
                bytes: std::sync::Arc::from(ics.as_bytes()),
            };
            s.bodies.insert("m3".into(), merel_engine::Body { text: "Geaccepteerd".into(), attachments: vec![part], ..Default::default() });
            s.selected_message = Some("m3".into());
        }
        h.run();
        h.get_by_label("Sanne heeft geaccepteerd: Lunch");
        assert!(h.query_all_by_label_contains("📎 bijlage").count() == 0, "the calendar part is the strip, not a file");
        assert!(h.query_all_by_label_contains("Deze vergadering staat niet in je agenda.").count() >= 1);
        assert!(h.query_by_label("Aan agenda toevoegen").is_none(), "an answer is not an invitation");
    }

    #[test]
    fn a_past_appointment_says_so() {
        let mut h = harness();
        let mut e = calendar::Editor::new_at(chrono::Local::now().naive_local() - chrono::TimeDelta::days(2));
        e.title = "Gisteren".into();
        h.state_mut().calendar.editor = Some(e);
        h.run();
        h.get_by_label(&calendar::in_the_past());
    }

    #[test]
    fn a_contact_group_is_made_mailed_and_offered_while_an_address_is_typed() {
        let mut h = harness();
        h.state_mut().module = Module::People;
        h.run();
        h.get_by_label(&groups::new()).click();
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
        h.get_by_label(&groups::mail_label()).click();
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
        assert_eq!((e.id.as_deref(), e.name().as_str(), e.emails.as_slice()), (None, "Nieuwsbrief", ["news@example.org".to_string()].as_slice()));
        h.get_by_label("Opslaan").click();
        h.run();
        assert_eq!(h.state().engine.session.contact_with_email("news@example.org").map(|c| c.name.as_str()), Some("Nieuwsbrief"));
    }

    #[test]
    fn an_urgent_message_gets_the_strip_over_it_and_a_normal_one_does_not() {
        let mut h = harness();
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        h.get_by_label(&importance::high_strip());
        h.get_by_label("Re: Offerte drukwerk").click();
        h.run();
        assert!(h.query_by_label(&importance::high_strip()).is_none() && h.query_by_label(&importance::low_strip()).is_none());
    }

    #[test]
    fn a_message_with_signs_of_phishing_gets_a_warning_strip_that_can_block_the_sender() {
        use merel_model::{Address, Flags, MessageHeader};
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
            chat: None,
            sealed: Default::default(),
        };
        let body = merel_engine::Body {
            rich: Some(merel_html::parse(r#"<p><a href="https://evil.example/login">https://www.bank.nl/inloggen</a></p>"#)),
            attachments: vec![merel_mime::Attachment {
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
        h.get_by_label(&phishing_title());
        assert!(h.query_by_label(&attachments::save_all_label()).is_none(), "one attachment needs no save-all button");
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
        assert!(h.query_by_label(&phishing_title()).is_none());
    }

    #[test]
    fn a_picture_attachment_shows_a_thumbnail_that_opens_the_preview_window() {
        let mut h = harness();
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf,
            0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xc9, 0xfe, 0x92, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        let picture = merel_mime::Attachment { name: "foto.png".into(), mime: "image/png".into(), size: png.len(), bytes: std::sync::Arc::from(png) };
        let paper =
            merel_mime::Attachment { name: "bon.pdf".into(), mime: "application/pdf".into(), size: 3, bytes: std::sync::Arc::from(&b"abc"[..]) };
        assert!(attachments::previewable(&picture) && !attachments::previewable(&paper));
        let body = merel_engine::Body { text: "Zie bijlage".into(), attachments: vec![paper, picture], ..Default::default() };
        h.state_mut().engine.session.bodies.insert("m3".into(), body);
        h.state_mut().engine.session.selected_message = Some("m3".into());
        h.run();
        h.get_by_label_contains("📎 bon.pdf");
        h.get_by_label(&attachments::save_all_label());
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
    fn pictures_from_a_safe_sender_load_without_asking_once_the_person_says_always() {
        let mut h = harness();
        let body =
            merel_engine::Body { rich: Some(merel_html::parse(r#"<p><img src="https://t.example/p.gif" alt="pixel"></p>"#)), ..Default::default() };
        h.state_mut().engine.session.bodies.insert("m1".into(), body);
        h.state_mut().engine.session.selected_message = Some("m1".into());
        h.run();
        h.get_by_label("Afbeeldingen laden");
        h.get_by_label(&always_from_sender()).click();
        h.run();
        let from = h.state().engine.session.message("m1").expect("sample mail").from.email().to_string();
        assert!(h.state().engine.session.junk.is_safe(&from), "the sender is on the safe list");
        assert!(h.state().remote.allowed("m1"));
        h.get_by_label_contains(&safe_images_notice());
        // Another message, then back: the pictures load without a click.
        h.state_mut().engine.session.selected_message = Some("m2".into());
        h.run();
        assert!(!h.state().remote.allowed("m1"));
        h.state_mut().engine.session.selected_message = Some("m1".into());
        h.run();
        assert!(h.state().remote.allowed("m1"), "a safe sender's pictures load on their own");
        assert!(h.query_by_label("Afbeeldingen laden").is_none());
    }

    #[test]
    fn links_attachments_and_pictures_are_switched_off_in_the_junk_folder() {
        use egui_kittest::kittest::NodeT;
        let mut h = harness();
        let body = merel_engine::Body {
            rich: Some(merel_html::parse(
                r#"<p><a href="https://example.nl/actie">Klik hier</a> <img src="https://t.example/p.gif" alt="pixel"></p>"#,
            )),
            attachments: vec![merel_mime::Attachment {
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
        h.get_by_label(&junk_notice());
        assert!(h.get_by_label_contains("bon.pdf").accesskit_node().is_disabled(), "the attachment cannot be saved from here");
        assert!(h.query_by_label("Afbeeldingen laden").is_none(), "pictures on the internet cannot be asked for");
        h.get_by_label("Klik hier").hover();
        h.run();
        assert!(h.query_by_label_contains(&html::links_off()).is_some(), "the link says why it does nothing");

        // "Geen ongewenst" brings the message back, and everything works again.
        h.get_by_label("Geen ongewenst").click();
        h.run();
        assert_eq!(h.state().engine.session.message("m3").map(|m| m.folder_id.as_str()), Some("inbox"));
        // The move dropped the selection, as any move does; back on the message everything works.
        h.state_mut().engine.session.selected_message = Some("m3".into());
        h.run();
        assert!(h.query_by_label(&junk_notice()).is_none());
        assert!(!h.get_by_label_contains("bon.pdf").accesskit_node().is_disabled());
        // "Geen ongewenst" put the sender on the safe list, so the pictures now load unasked.
        assert!(h.query_by_label("Afbeeldingen laden").is_none());
        h.get_by_label_contains(&safe_images_notice());
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
        h.get_by_label("Opvolgen: Wat er deze maand speelt");
        h.get_by_label("Vandaag");
        assert!(h.state().announced_reminders.iter().any(|(k, _)| k == "followup:m3"), "the window asked for attention once");
        let alerts: Vec<_> = h.state().alerts.iter().filter(|a| a.id == "mail:m3").collect();
        assert_eq!(alerts.len(), 1, "one notification for the computer, not one per frame");
        assert_eq!(alerts[0].title, "Herinnering", "no subject in the notification unless asked");
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.query_by_label("Opvolgen: Wat er deze maand speelt").is_none(), "closed; the flag stays");
        assert!(h.state().engine.session.message("m3").unwrap().flags.flagged);
        assert!(h.state().engine.session.follow_ups.iter().any(|f| f.dismissed));
    }

    #[test]
    fn a_task_whose_due_day_has_come_shows_in_the_reminders_window() {
        use chrono::{Days, Local, NaiveTime, TimeZone};
        let mut h = harness();
        h.run();
        assert!(h.query_by_label_contains("Herinnering").is_none(), "the sample tasks do not nag");
        let yesterday = Local::now().date_naive().checked_sub_days(Days::new(1)).unwrap();
        let due = Local.from_local_datetime(&yesterday.and_time(NaiveTime::MIN)).earliest().unwrap().timestamp();
        h.state_mut().run("task.save", json!({ "title": "Belastingaangifte", "due": due }));
        h.run();
        h.get_by_label("Taak: Belastingaangifte");
        h.get_by_label("Gisteren");
        assert!(h.state().announced_reminders.iter().any(|(k, _)| k.starts_with("task:")), "the window asked for attention once");
        h.get_by_label("Openen").click();
        h.run();
        assert_eq!(h.state().module, Module::Tasks);
        assert_eq!(h.state().engine.session.selected_task().map(|t| t.title.as_str()), Some("Belastingaangifte"));
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.query_by_label("Taak: Belastingaangifte").is_none(), "closed; the task stays open");
        assert!(h.state().engine.session.tasks.iter().any(|t| t.title == "Belastingaangifte" && !t.done));
        let id = h.state().engine.session.tasks.iter().find(|t| t.title == "Belastingaangifte").map(|t| t.id.clone()).unwrap();
        assert!(h.state().engine.session.tasks_data().dismissed.iter().any(|k| k.id == id && k.due == due), "closing is kept for this due day");

        // Back in the mail module, so the task module's own "Voltooid" filter is not on screen.
        h.state_mut().module = Module::Mail;
        h.state_mut().run("task.save", json!({ "title": "Verslag", "due": due }));
        h.run();
        h.get_by_label("Taak: Verslag");
        h.get_by_label("Voltooid").click();
        h.run();
        assert!(h.query_by_label("Taak: Verslag").is_none());
        assert!(h.state().engine.session.tasks.iter().any(|t| t.title == "Verslag" && t.done), "Voltooid ticks the task off");
    }

    #[test]
    fn an_invitation_in_the_mail_opens_the_appointment_window_filled_in() {
        let mut h = harness();
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:k1@x\r\nDTSTART:20261021T080000Z\r\nDTEND:20261021T093000Z\r\nSUMMARY:Kickoff\r\nLOCATION:Kantoor\r\nORGANIZER;CN=Sanne:mailto:sanne@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let body = merel_engine::Body {
            text: "Kom je ook?".into(),
            attachments: vec![merel_mime::Attachment {
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
            let body = merel_engine::Body {
                text: "Nieuws".into(),
                unsubscribe: vec!["https://example.org/u".into(), "mailto:leave@example.org?subject=stop".into()],
                ..Default::default()
            };
            s.bodies.insert("m3".into(), body);
            s.selected_message = Some("m3".into());
        }
        h.run();
        h.get_by_label(&unsubscribe::strip());
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
    fn a_request_for_a_read_receipt_shows_a_strip_and_sends_only_on_a_click() {
        let mut h = harness();
        {
            let s = &mut h.state_mut().engine.session;
            let body = merel_engine::Body {
                text: "Gelezen?".into(),
                receipt_to: Some(merel_model::Address::new(Some("Sanne"), "sanne@example.org").unwrap()),
                ..Default::default()
            };
            s.bodies.insert("m3".into(), body);
            s.selected_message = Some("m3".into());
        }
        h.run();
        h.get_by_label(&receipt::strip_label());
        assert!(h.state().engine.session.outbox.is_empty(), "nothing goes out by itself");
        h.get_by_label(&receipt::send_label()).click();
        h.run();
        let s = &h.state().engine.session;
        assert_eq!(s.outbox.len(), 1, "the receipt was handed to the outbox");
        assert!(h.state().notice.is_some(), "the sample account cannot send, and the person hears so");
    }

    #[test]
    fn a_broken_signature_shows_a_warning_over_the_message() {
        let mut h = harness();
        {
            let s = &mut h.state_mut().engine.session;
            let body =
                merel_engine::Body { text: "Betaal deze factuur".into(), signature: Some(merel_smime::Signature::Broken), ..Default::default() };
            s.bodies.insert("m3".into(), body);
            s.selected_message = Some("m3".into());
        }
        h.run();
        h.get_by_label(&signature::broken());
    }

    #[test]
    fn a_certificate_from_an_unknown_issuer_can_be_trusted_from_the_strip() {
        let mut h = harness();
        {
            let s = &mut h.state_mut().engine.session;
            let from = s.message("m3").unwrap().from.email().to_lowercase();
            let signer = merel_smime::CertInfo { name: "Iemand".into(), emails: vec![from], issuer: "Eigen CA".into(), not_before: 0, not_after: 0 };
            let signature = merel_smime::Signature::Valid { signer, certificate: vec![1, 2, 3], issuers: Vec::new(), trusted: false };
            s.bodies.insert("m3".into(), merel_engine::Body { text: "Hoi".into(), signature: Some(signature), ..Default::default() });
            s.selected_message = Some("m3".into());
        }
        h.run();
        assert_eq!(h.query_all_by_label_contains("is niet bekend als betrouwbaar").count(), 1);
        h.get_by_label(&signature::trust_label()).click();
        h.run();
        assert!(h.query_by_label(&signature::trust_label()).is_none());
        assert_eq!(h.query_all_by_label_contains("✔ Digitaal ondertekend door Iemand").count(), 1);
    }

    #[test]
    fn an_encrypted_message_says_so_over_its_text() {
        let mut h = harness();
        {
            let s = &mut h.state_mut().engine.session;
            let body = merel_engine::Body { text: "Het contract".into(), encrypted: Some(merel_mime::Encryption::Opened), ..Default::default() };
            s.bodies.insert("m3".into(), body);
            s.selected_message = Some("m3".into());
        }
        h.run();
        h.get_by_label(&signature::encrypted());
    }

    #[test]
    fn flagged_mail_shows_in_the_tasks_module_opens_in_mail_and_is_done_from_its_checkbox() {
        let mut h = harness();
        // Tomorrow, not today: a due follow-up would open the reminders window over the list.
        h.state_mut().run("message.follow_up", json!({ "when": "tomorrow", "id": "m1" }));
        h.state_mut().run("message.flag", json!({ "id": "m3" }));
        h.state_mut().module = Module::Tasks;
        h.run();
        h.get_by_label(&tasks::flagged_heading());
        assert!(h.query_by_label("Planning voor donderdag").is_some() && h.query_by_label("Wat er deze maand speelt").is_some());
        h.get_by_label("Voltooid").click();
        h.run();
        assert!(h.query_by_label(&tasks::flagged_heading()).is_none(), "the done filter is about tasks; flagged mail is open by nature");
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
        assert_eq!(s.follow_up("m1").and_then(merel_engine::followup::due_day), Some(tomorrow));
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
        h.get_by_label(&multi_hint());

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
    fn recente_zoekopdrachten_offer_the_last_words_typed_and_can_be_cleared() {
        let mut h = harness();
        h.run();
        assert!(h.query_by_label(&view::recent_menu()).is_none(), "no menu before anything was searched");
        h.state_mut().view.recent_searches = vec!["offerte".into(), "factuur".into()];
        h.run();
        h.get_by_label(&view::recent_menu()).click();
        h.run();
        h.get_by_label("factuur").click();
        h.run();
        assert_eq!(h.state().engine.session.search, "factuur");
        assert_eq!(h.state().view.recent_searches, ["factuur", "offerte"], "the one picked moves to the top");
        h.get_by_label(&view::recent_menu()).click();
        h.run();
        h.get_by_label(&view::recent_clear()).click();
        h.run();
        assert!(h.state().view.recent_searches.is_empty());
        assert!(h.query_by_label(&view::recent_menu()).is_none());
    }

    #[test]
    fn a_search_is_saved_as_a_search_folder_and_run_again_from_the_folder_pane() {
        let mut h = harness();
        h.run();
        assert!(h.query_by_label(&search_folders::heading()).is_none());
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
        h.get_by_label(&search_folders::heading());

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
        assert!(h.state().view.search_folders.is_empty() && h.query_by_label(&search_folders::heading()).is_none());
    }

    /// Two connected accounts, each with an inbox holding one unread message.
    fn two_accounts() -> egui_kittest::Harness<'static, MailApp> {
        use merel_engine::{Event, RemoteFolder, RemoteHeader};
        let mut engine = Engine::sample();
        for (email, subject) in [("alain@voorbeeld.nl", "Van het eerste"), ("rees@voorbeeld.org", "Van het tweede")] {
            let mut f = crate::backstage::AccountForm::default();
            f.email = email.into();
            f.password = "x".into();
            f.suggest();
            let account = f.account().expect("account");
            let id = account.id.clone();
            engine.add_account(account, merel_secrets::Secret::new("x")).expect("add");
            let inbox =
                RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: merel_model::FolderKind::Inbox, selectable: true, parent: None };
            engine.session.apply(Event::Connected { account_id: id.clone(), folders: vec![inbox], syncs_items: false });
            let header = RemoteHeader {
                id: "1".into(),
                seen: false,
                flagged: false,
                answered: false,
                size: 1,
                has_attachments: false,
                categories: Vec::new(),
                header: merel_mime::parse_header(format!("From: iemand@voorbeeld.nl\r\nSubject: {subject}\r\n\r\n").as_bytes()),
                date: 1_700_000_000,
            };
            engine.session.apply(Event::Headers { folder_id: format!("{id}/INBOX"), headers: vec![header], offset: 0, limit: 200 });
        }
        engine.session.take_jobs();
        engine.session.loading.clear();
        egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), MailApp::new(engine))
    }

    #[test]
    fn all_accounts_lists_the_mail_of_every_account_and_folds_shut() {
        let mut h = two_accounts();
        h.run();
        h.get_by_label("Postvak IN (alle accounts)").click();
        h.run();
        h.get_by_label("Van het eerste");
        h.get_by_label("Van het tweede");
        // A reply from the unified inbox goes out from the account the message came to.
        let second = h.state().engine.session.messages.iter().find(|m| m.subject == "Van het tweede").map(|m| m.id.clone()).expect("listed");
        h.state_mut().select_message(&second);
        let account = h.state().engine.session.current_account().map(|a| a.address.email().to_string());
        assert_eq!(account.as_deref(), Some("rees@voorbeeld.org"));

        // The body is on its way, so the window repaints on a timer: a bounded number of steps.
        h.get_by_label(merel_engine::unified::HEADING).click();
        h.run_steps(2);
        assert!(h.query_by_label("Postvak IN (alle accounts)").is_none());
        assert!(h.state().view.all_accounts_closed);
    }

    #[test]
    fn subfolders_nest_under_their_parent_and_start_folded_shut() {
        let folder = |id: &str, parent: Option<&str>| merel_model::Folder {
            id: id.into(),
            account_id: "a".into(),
            name: id.rsplit('/').next().unwrap_or(id).into(),
            path: id.into(),
            kind: FolderKind::Other,
            unread: 0,
            parent: parent.map(str::to_string),
        };
        let folders = [
            folder("Inbox", None),
            folder("Prive", None),
            folder("Prive/HUIS", Some("Prive")),
            folder("Prive/HUIS/2026", Some("Prive/HUIS")),
            folder("Weg/Los", Some("Weg")),
        ];
        let rows = |open: &[&str]| -> Vec<(String, usize, Option<bool>)> {
            let open: Vec<String> = open.iter().map(|s| s.to_string()).collect();
            folder_tree(&folders, &open).into_iter().map(|(f, d, o)| (f.id, d, o)).collect()
        };
        assert_eq!(
            rows(&[]),
            [("Inbox".into(), 0, None), ("Prive".into(), 0, Some(false)), ("Weg/Los".into(), 0, None)],
            "shut by default; a folder whose parent is not listed stands at the top"
        );
        assert_eq!(
            rows(&["Prive", "Prive/HUIS"]),
            [
                ("Inbox".into(), 0, None),
                ("Prive".into(), 0, Some(true)),
                ("Prive/HUIS".into(), 1, Some(true)),
                ("Prive/HUIS/2026".into(), 2, None),
                ("Weg/Los".into(), 0, None)
            ]
        );
    }

    #[test]
    fn an_account_folds_shut_and_a_new_subfolder_shows_under_its_parent() {
        let mut h = two_accounts();
        h.run();
        let inboxes = h.query_all_by_label("Postvak IN").count();
        h.get_by_label("alain@voorbeeld.nl").click();
        h.run();
        assert_eq!(h.query_all_by_label("Postvak IN").count(), inboxes - 1, "the first account is folded shut");
        assert_eq!(h.state().view.closed_accounts.len(), 1);
        h.get_by_label("alain@voorbeeld.nl").click();
        h.run();
        assert_eq!(h.query_all_by_label("Postvak IN").count(), inboxes);

        let mut h = harness();
        h.run();
        h.state_mut().new_folder(Some("inbox".into()));
        h.state_mut().folder_edit.as_mut().expect("edit").name = "Projecten".into();
        h.run();
        h.get_by_label("Map aanmaken").click();
        h.run();
        h.get_by_label("Projecten");
        h.get_by_label("Inklappen: Postvak IN").click();
        h.run();
        assert!(h.query_by_label("Projecten").is_none(), "folded shut");
        h.get_by_label("Uitklappen: Postvak IN").click();
        h.run();
        h.get_by_label("Projecten");
    }

    #[test]
    fn one_account_has_no_all_accounts_group() {
        let mut h = harness();
        h.run();
        assert!(h.query_by_label(merel_engine::unified::HEADING).is_none());
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
        use merel_model::{Flags, MessageHeader};
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
        h.get_by_label(&ignored_notice());
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
        assert_eq!(h.state().engine.session.junk.level, merel_model::JunkLevel::High);
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
    fn egui_draws_its_own_text_placeholders_and_disabled_widgets_in_the_readable_colours() {
        let mut h = harness();
        h.run();
        h.step();
        let t = theme::Tokens::light();
        let visuals = h.ctx.style_of(egui::Theme::Light).visuals.clone();
        assert_eq!(visuals.weak_text_color(), t.text_dim, "placeholders such as the search box hint use text_dim");
        assert_eq!(visuals.widgets.noninteractive.fg_stroke.color, t.text, "plain labels use text");
        assert_eq!(visuals.widgets.inactive.fg_stroke.color, t.text, "buttons use text");
        assert_eq!(visuals.widgets.inactive.bg_stroke.color, t.control_border, "controls get a visible border");
        assert_eq!(visuals.disabled_alpha, theme::DISABLED_ALPHA);
        h.state_mut().view.theme = view::ThemeChoice::Dark;
        h.state_mut().theme_applied = false;
        h.run();
        h.step();
        assert_eq!(
            h.ctx.style_of(egui::Theme::Dark).visuals.weak_text_color(),
            theme::Tokens::dark().text_dim,
            "the dark theme carries its own colours"
        );
    }

    #[test]
    fn every_symbol_the_ui_draws_has_a_glyph_in_the_apps_fonts() {
        let mut h = harness();
        h.run();
        h.step();
        let dirs = [concat!(env!("CARGO_MANIFEST_DIR"), "/src"), concat!(env!("CARGO_MANIFEST_DIR"), "/../i18n/locales")];
        // The atlas rectangle a character is drawn from. A character no font has gets the
        // replacement box, so the same rectangle as a CJK character means an empty box on screen.
        let drawn = |h: &mut egui_kittest::Harness<'static, MailApp>, c: char| {
            h.ctx.fonts_mut(|f| f.layout_no_wrap(c.to_string(), egui::FontId::proportional(14.0), Color32::WHITE)).rows[0].row.glyphs[0].uv_rect
        };
        let replacement = drawn(&mut h, '\u{4E00}');
        assert_eq!(drawn(&mut h, '\u{4E01}'), replacement, "two characters no font has draw the same box");
        assert_ne!(drawn(&mut h, 'a'), replacement);
        let mut missing = std::collections::BTreeSet::new();
        for entry in dirs.iter().flat_map(|dir| std::fs::read_dir(dir).expect("source directory")) {
            let path = entry.expect("entry").path();
            if path.extension().is_some_and(|e| e == "rs" || e == "ftl") {
                let text = std::fs::read_to_string(&path).expect("source file");
                for c in text.chars().filter(|c| !c.is_ascii() && !matches!(c, '\u{200D}' | '\u{FE0F}')) {
                    if drawn(&mut h, c) == replacement {
                        missing.insert(c);
                    }
                }
            }
        }
        assert!(missing.is_empty(), "these characters would draw as empty boxes: {missing:?}");
    }

    /// Every label on screen, for the checks that look for words left in the wrong language.
    fn labels(h: &egui_kittest::Harness<'static, MailApp>) -> Vec<String> {
        use egui_kittest::kittest::NodeT;
        h.root().query_all(egui_kittest::kittest::by()).filter_map(|n| n.accesskit_node().label()).filter(|l| !l.trim().is_empty()).collect()
    }

    #[test]
    fn the_mail_screens_speak_english_in_english() {
        let mut h = harness();
        h.state_mut().view.language = i18n::LanguageChoice::English;
        h.state_mut().apply_language();
        h.state_mut().run("message.select", json!({ "id": "m1" }));
        h.run();
        h.run();
        let mut seen = labels(&h);
        h.state_mut().open_compose(ComposeKind::Reply);
        h.run();
        h.run();
        seen.extend(labels(&h));
        // A Dutch text from nl.ftl whose English differs must not show; the sample mail itself is
        // Dutch, so its subjects, names and text are left out.
        let dutch: std::collections::HashSet<String> = i18n::dutch_only_texts();
        let sample = ["Planning voor donderdag", "Sanne", "Merel", "Overleg ontwerp", "Voorbeeld"];
        let left: Vec<&String> = seen.iter().filter(|l| dutch.contains(l.as_str()) && !sample.iter().any(|s| l.contains(s))).collect();
        assert!(left.is_empty(), "Dutch on the English mail screens: {left:?}");
    }

    #[test]
    fn the_calendar_contacts_and_tasks_speak_english_in_english() {
        let mut h = harness();
        h.state_mut().view.language = i18n::LanguageChoice::English;
        h.state_mut().apply_language();
        let mut seen = Vec::new();
        let today = chrono::Local::now().date_naive();
        let screens: [&dyn Fn(&mut MailApp); 6] = [
            &|app| app.module = Module::Calendar,
            &|app| app.new_appointment(today, 9),
            &|app| {
                app.calendar.editor = None;
                app.module = Module::People;
            },
            &|app| app.new_contact(),
            &|app| app.module = Module::Tasks,
            &|app| app.new_task(),
        ];
        for screen in screens {
            screen(h.state_mut());
            h.run();
            h.run();
            seen.extend(labels(&h));
        }
        let dutch = i18n::dutch_only_texts();
        let left: Vec<&String> = seen.iter().filter(|l| dutch.contains(l.as_str())).collect();
        assert!(left.is_empty(), "Dutch on the English calendar, contacts and tasks: {left:?}");
    }

    #[test]
    fn every_ribbon_fits_the_window_without_overlap_in_every_language() {
        for language in i18n::Language::ALL {
            let mut h = harness();
            i18n::set(language);
            let cases = [
                (Module::Mail, Tab::Home, "new-mail"),
                (Module::Mail, Tab::Folder, "folder-new"),
                (Module::Mail, Tab::SendReceive, "refresh"),
                (Module::Mail, Tab::View, "language-system"),
                (Module::Mail, Tab::Help, "help-shortcuts"),
                (Module::Calendar, Tab::Home, "new-appointment"),
                (Module::Calendar, Tab::View, "language-system"),
                (Module::People, Tab::Home, "new-contact"),
                (Module::Tasks, Tab::Home, "new-task"),
            ];
            for (module, tab, first) in cases {
                h.state_mut().module = module;
                h.state_mut().tab = tab;
                h.run();
                h.run();
                let row = h.get_by_label(&tr(first)).rect().center().y;
                let mut buttons: Vec<(String, egui::Rect)> = h
                    .query_all_by_role(egui::accesskit::Role::Button)
                    .map(|n| (egui_kittest::kittest::NodeT::accesskit_node(&n).label().unwrap_or_default(), n.rect()))
                    .filter(|(_, r)| (r.center().y - row).abs() < 4.0 && r.width() > 0.0)
                    .collect();
                buttons.sort_by(|a, b| a.1.min.x.total_cmp(&b.1.min.x));
                let at = format!("{module:?}/{tab:?} in {}", language.tag());
                assert!(!buttons.is_empty(), "{at}: the ribbon has buttons");
                for pair in buttons.windows(2) {
                    assert!(pair[0].1.max.x <= pair[1].1.min.x + 0.5, "{at}: {} runs into {}", pair[0].0, pair[1].0);
                }
                let (last, rect) = &buttons[buttons.len() - 1];
                assert!(rect.max.x <= 1280.0, "{at}: {last} ends at {} and falls off a 1280 px window", rect.max.x);
            }
        }
    }

    #[test]
    fn the_language_switches_on_the_view_tab_and_the_ribbon_still_fits() {
        let mut h = harness();
        h.state_mut().tab = Tab::View;
        h.run();
        assert!(h.query_by_label("Thema").is_some() && h.query_by_label("Taal").is_some(), "tests run in Dutch");
        let menu = h.get_by_label("Systeemtaal").rect();
        assert!(menu.max.x <= 1280.0, "the language menu ends at {} and falls off a 1280 px window", menu.max.x);
        h.get_by_label("Systeemtaal").click();
        h.run();
        h.get_by_label("English").click();
        h.run();
        h.run();
        assert_eq!(h.state().view.language, i18n::LanguageChoice::English);
        assert_eq!(i18n::current(), i18n::Language::English);
        assert!(h.query_by_label("Theme").is_some() && h.query_by_label("Dark").is_some(), "the theme buttons speak English now");
        let menu = h.get_by_label("English").rect();
        assert!(menu.max.x <= 1280.0, "the language menu ends at {} in English and falls off a 1280 px window", menu.max.x);
        h.get_by_label("English").click();
        h.run();
        h.get_by_label("Nederlands").click();
        h.run();
        h.run();
        assert!(h.query_by_label("Donker").is_some());
        assert_eq!(i18n::current(), i18n::Language::Dutch);
    }

    #[test]
    fn ribbon_buttons_are_as_wide_as_their_labels_and_the_ribbons_fit_the_window() {
        let mut h = harness();
        h.run();
        h.step();
        let reply = h.get_by_label("Beantwoorden").rect();
        let all = h.get_by_label("Allen beantwoorden").rect();
        let forward = h.get_by_label("Doorsturen").rect();
        assert!(reply.max.x <= all.min.x && all.max.x <= forward.min.x, "the three buttons sit side by side without overlap");
        assert!(all.width() > reply.width(), "the longer label gets the wider button");
        let label = h.ctx.fonts_mut(|f| f.layout_no_wrap("Allen beantwoorden".to_owned(), egui::FontId::proportional(12.0), Color32::WHITE));
        assert!(all.width() >= label.size().x + 2.0 * RIBBON_LABEL_PADDING, "the label fits inside its button");
        assert!(h.query_by_label("Reageren").is_some() && h.query_by_label("Labels").is_some(), "the groups carry their names");
        for (tab, last) in [(Tab::Home, "Snelle stappen"), (Tab::View, "Taal"), (Tab::Folder, "Opruimen")] {
            h.state_mut().tab = tab;
            h.run();
            let rect = h.get_by_label_contains(last).rect();
            assert!(rect.max.x <= 1280.0, "{last} on the {tab:?} tab ends at {} and falls off a 1280 px window", rect.max.x);
        }
    }

    #[test]
    fn two_calendars_show_merged_with_tabs_or_side_by_side() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.view = calendar::View::Day;
        h.run();
        assert!(h.query_by_label(&calendar::split_label("Mijn agenda")).is_none(), "one calendar needs no tabs");
        h.state_mut().engine.run("calendar.add", &json!({ "name": "Sport" })).unwrap();
        h.run();
        assert!(h.query_by_label(&calendar::split_label("Mijn agenda")).is_some() && h.query_by_label(&calendar::close_label("Sport")).is_some());
        let today = calendar::today();
        let day =
            format!("{} {} {}", calendar::weekday_name(today), chrono::Datelike::day(&today), calendar::month_name(chrono::Datelike::month(&today)));
        assert!(h.query_by_label(&day).is_some(), "merged, one column for the day");

        h.get_by_label(&calendar::split_label("Sport")).click();
        h.run();
        assert!(h.state().view.calendars_side_by_side);
        assert!(
            h.query_by_label(&format!("{day}, Mijn agenda")).is_some() && h.query_by_label(&format!("{day}, Sport")).is_some(),
            "a column per calendar"
        );
        let mine = h.get_by_label(&format!("{day}, Mijn agenda")).rect();
        let sport = h.get_by_label(&format!("{day}, Sport")).rect();
        assert!(mine.max.x <= sport.min.x + 1.0, "side by side, in the side pane's order");
        assert!(h.query_by_label(&calendar::merge_label("Sport")).is_some(), "the arrow now merges");

        // A click in Sport's own hours makes the appointment there.
        h.get_by_label(&format!("{day}, Sport")).click();
        h.run();
        let sport_id = h.state().engine.session.calendars.iter().find(|c| c.name == "Sport").unwrap().id.clone();
        assert_eq!(h.state().calendar.editor.as_ref().and_then(|e| e.calendar_id.clone()), Some(sport_id.clone()));
        h.state_mut().calendar.editor = None;
        h.run();

        h.get_by_label(&calendar::close_label("Sport")).click();
        h.run();
        assert!(!h.state().view.calendar_shown(&sport_id), "the cross unticks the calendar");
        assert!(h.query_by_label(&format!("{day}, Mijn agenda")).is_none() && h.query_by_label(&day).is_some(), "one calendar left, one grid");
        h.get_by_label("Sport").click();
        h.run();
        h.get_by_label(&calendar::merge_label("Mijn agenda")).click();
        h.run();
        assert!(!h.state().view.calendars_side_by_side, "merged again");
    }

    #[test]
    fn an_appointment_gets_and_loses_categories_in_its_window() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.query = "Lunch met Sanne".into();
        h.run();
        let id = h.state().engine.session.appointments.iter().find(|a| a.title == "Lunch met Sanne").unwrap().id.clone();
        let start = h.state().engine.session.appointment(&id).unwrap().start;
        h.state_mut().open_appointment(&id, start);
        h.run();
        h.get_by_label(&calendar::categorize()).click();
        h.run();
        h.get_by_label("● Rode categorie").click();
        h.run();
        assert!(h.query_by_label("● Groene categorie").is_none(), "a pick closes the menu");
        h.get_by_label(&calendar::categorize()).click();
        h.run();
        h.get_by_label("● Groene categorie").click();
        h.run();
        assert_eq!(h.state().calendar.editor.as_ref().unwrap().categories, vec!["Rode categorie", "Groene categorie"]);
        h.get_by_label(&calendar::remove_category_label("Rode categorie")).click();
        h.run();
        assert_eq!(h.state().calendar.editor.as_ref().unwrap().categories, vec!["Groene categorie"], "the chip in the form takes it off");
        h.get_by_label("Opslaan").click();
        h.run();
        assert_eq!(h.state().engine.session.appointment(&id).unwrap().categories, vec!["Groene categorie"]);
        let a = h.state().engine.session.appointment(&id).unwrap().clone();
        let green = crate::categories::color_of(merel_model::CategoryColor::Green);
        assert_eq!(calendar::block_color(h.state(), &a, &Tokens::light()), green, "a categorised block takes its category's colour");
        let start = h.state().engine.session.appointment(&id).unwrap().start;
        h.state_mut().open_appointment(&id, start);
        h.run();
        h.get_by_label(&calendar::categorize()).click();
        h.run();
        h.get_by_label(&calendar::clear_categories()).click();
        h.run();
        assert!(h.state().calendar.editor.as_ref().unwrap().categories.is_empty(), "Alle categorieën wissen empties the list");
    }

    #[test]
    fn a_calendar_is_added_given_a_colour_renamed_and_deleted_from_the_side_pane() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        h.get_by_label(&calendars::add()).click();
        h.run();
        h.get_by_label("Kleur Groen").click();
        h.run();
        h.state_mut().calendar.dialog = Some(calendars::Dialog::Add { name: "Sport".into(), color: [0x2E, 0x8B, 0x57] });
        h.run();
        h.get_by_label("Toevoegen").click();
        h.run();
        assert!(h.state().calendar.dialog.is_none(), "the window closes once the calendar is there");
        let sport = h.state().engine.session.calendars.iter().find(|c| c.name == "Sport").cloned().unwrap();
        assert_eq!(sport.color, [0x2E, 0x8B, 0x57]);
        assert!(h.query_by_label("Sport").is_some(), "the new calendar is listed");

        h.get_by_label(&calendars::options_label("Mijn agenda")).click();
        h.run();
        h.get_by_label("Kleur Rood").click();
        h.run();
        assert_eq!(h.state().engine.session.calendar("local").unwrap().color, [0xD4, 0x46, 0x46], "a colour from the menu is the calendar's");

        h.get_by_label(&calendars::options_label("Sport")).click();
        h.run();
        h.get_by_label("Naam wijzigen…").click();
        h.run();
        assert!(matches!(h.state().calendar.dialog, Some(calendars::Dialog::Rename { .. })));
        h.state_mut().calendar.dialog = Some(calendars::Dialog::Rename { id: sport.id.clone(), name: "Hardlopen".into() });
        h.run();
        h.get_by_label("Opslaan").click();
        h.run();
        assert_eq!(h.state().engine.session.calendar(&sport.id).unwrap().name, "Hardlopen");

        h.get_by_label(&calendars::options_label("Hardlopen")).click();
        h.run();
        h.get_by_label("Verwijderen…").click();
        h.run();
        assert!(h.query_by_label_contains("Agenda \"Hardlopen\" verwijderen?").is_some(), "deleting asks first");
        h.get_by_label(&calendars::confirm_delete()).click();
        h.run();
        assert!(h.state().engine.session.calendar(&sport.id).is_none());
        h.get_by_label(&calendars::options_label("Mijn agenda")).click();
        h.run();
        assert!(h.query_by_label("Verwijderen…").is_none(), "the last calendar on this computer cannot be deleted");
    }

    #[test]
    fn calendars_stand_under_the_address_of_their_account_and_local_ones_under_their_own_heading() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        // Two accounts with the same display name, as with one person's private and work mail.
        let first = h.state().engine.session.accounts[0].clone();
        let mut second = first.clone();
        second.id = "werk".into();
        second.address = merel_model::Address::new(None, "alain@werk.example").unwrap();
        h.state_mut().engine.session.accounts.push(second);
        for a in ["sample", "werk"] {
            h.state_mut().engine.session.calendars.push(merel_model::Calendar {
                id: format!("{a}/calendar"),
                name: merel_engine::sync::ACCOUNT_CALENDAR.into(),
                account_id: Some(a.into()),
                color: [0x2E, 0x8B, 0x57],
            });
        }
        h.run();
        let local = h.get_by_label(&calendars::local()).rect();
        let mine = h.get_by_label("Mijn agenda").rect();
        let own = h.get_by_label(first.address.email()).rect();
        let work = h.get_by_label("alain@werk.example").rect();
        let agendas: Vec<egui::Rect> =
            h.query_all_by_label("Agenda").map(|n| n.rect()).filter(|r| r.left() < local.right() && r.top() > local.top()).collect();
        assert_eq!(agendas.len(), 2, "each account's calendar is listed once in the side pane");
        assert!(local.top() < mine.top() && mine.top() < own.top(), "the local calendar under its heading, before the accounts");
        assert!(own.top() < agendas[0].top() && agendas[0].top() < work.top() && work.top() < agendas[1].top(), "each calendar under its account");
        assert!(h.query_by_label("Mijn agenda's").is_none());
    }

    #[test]
    fn unticking_a_calendar_hides_its_appointments_until_it_is_ticked_again() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.query = "sanne".into();
        h.run();
        assert!(h.query_by_label("Lunch met Sanne").is_some(), "the appointment shows while its calendar is ticked");
        h.get_by_label("Mijn agenda").click();
        h.run();
        assert_eq!(h.state().view.hidden_calendars, vec!["local".to_string()], "the calendar is remembered as hidden");
        assert!(h.query_by_label("Lunch met Sanne").is_none(), "the appointment is gone while its calendar is unticked");
        assert!(h.query_by_label_contains("Geen afspraak gevonden").is_some());
        h.get_by_label("Mijn agenda").click();
        h.run();
        assert!(h.state().view.hidden_calendars.is_empty());
        assert!(h.query_by_label("Lunch met Sanne").is_some(), "ticking the calendar again brings the appointment back");
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
    fn the_buttons_of_the_appointment_window_never_overlap() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.view = calendar::View::Week;
        h.run();
        let focus = h.state().calendar.focus;
        h.state_mut().calendar.focus = focus.checked_add_days(chrono::Days::new(7)).expect("next week");
        h.run();
        h.get_by_label_contains("Weekstart").click();
        h.run();
        {
            // The widest the bar gets: an invitation to send, the longest reminder and status.
            let e = h.state_mut().calendar.editor.as_mut().expect("the series opens");
            e.attendees = "sanne@example.org".into();
            e.reminder = Some(0);
            e.show_as = merel_model::ShowAs::Tentative;
            e.private = true;
        }
        h.run();
        h.run();
        // Left to right, each button ends before the next one starts.
        let rects = [
            h.get_by_label("Verzenden").rect(),
            h.get_by_label_contains("Voorlopig").rect(),
            h.get_by_label("Op het moment zelf").rect(),
            h.get_by_label(&calendar::private()).rect(),
            h.get_by_label(&crate::meeting::forward_label()).rect(),
            h.get_by_label(&calendar::delete_menu()).rect(),
            h.get_by_label("Annuleren").rect(),
        ];
        for pair in rects.windows(2) {
            assert!(pair[0].max.x <= pair[1].min.x + 0.5, "{:?} runs into {:?}", pair[0], pair[1]);
        }
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
        assert_eq!(editor.repeat, Some(merel_model::Frequency::Weekly));
        let clicked = editor.occurrence.expect("the clicked occurrence is remembered");
        assert!(clicked > series_start && clicked - series_start <= 8 * 86_400, "it is next week's, not the first");
        h.get_by_label(&calendar::delete_menu()).click();
        h.run();
        h.get_by_label(&calendar::skip_one()).click();
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
            e.repeat = Some(merel_model::Frequency::Daily);
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
        h.get_by_label("Welkom bij Merel").click();
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
        assert!(h.query_by_label("Welkom bij Merel").is_none(), "gone from the inbox list");
        h.get_by_label("Archief").click();
        h.run();
        h.get_by_label("Welkom bij Merel");
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
    fn an_appointment_in_the_day_view_opens_with_a_double_click_and_from_its_right_click_menu() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.view = calendar::View::Day;
        h.state_mut().calendar.focus = calendar::today();
        h.run();
        // A double click, as in Outlook, opens the appointment in its window.
        let block = h.get_by_label_contains("Overleg ontwerp\n");
        block.click();
        block.click();
        h.run();
        let e = h.state().calendar.editor.as_ref().expect("a double click opens the appointment");
        assert_eq!((e.id.as_deref(), e.title.as_str()), (Some("sample-0"), "Overleg ontwerp"));
        assert!(h.query_by_label("Opslaan").is_some(), "the window shows");
        h.state_mut().calendar.editor = None;
        h.run();

        // A right click offers Openen and Verwijderen.
        h.get_by_label_contains("Overleg ontwerp\n").click_secondary();
        h.run();
        assert!(h.state().calendar.editor.is_none(), "a right click only shows the menu");
        h.get_by_label(&calendar::open_menu()).click();
        h.run();
        assert_eq!(h.state().calendar.editor.as_ref().and_then(|e| e.id.as_deref()), Some("sample-0"), "Openen opens it");
        h.state_mut().calendar.editor = None;
        h.run();

        let block = h.get_by_label_contains("Overleg ontwerp\n").rect();
        h.get_by_label_contains("Overleg ontwerp\n").click_secondary();
        h.run();
        // The menu's, not the ribbon's, under the block where it was clicked.
        h.query_all_by_label("Verwijderen").find(|n| n.rect().top() > block.top()).expect("Verwijderen in the menu").click();
        h.run();
        assert!(h.state().engine.session.appointment("sample-0").is_none(), "Verwijderen deletes it");
        assert!(h.state().calendar.editor.is_none());
    }

    #[test]
    fn the_right_click_menu_of_a_series_deletes_one_occurrence_or_all() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.state_mut().calendar.view = calendar::View::Month;
        h.state_mut().calendar.focus = calendar::today();
        h.run();
        let series = |h: &egui_kittest::Harness<'static, MailApp>| h.query_all_by_label_contains("Weekstart").count();
        let before = series(&h);
        assert!(before > 1, "the weekly sample shows more than once in the month");
        h.query_all_by_label_contains("Weekstart").next().expect("a Weekstart").click_secondary();
        h.run();
        assert_eq!(h.query_all_by_label("Verwijderen").count(), 1, "only the ribbon's: a series asks which");
        h.get_by_label(&calendar::delete_one_menu()).click();
        h.run();
        assert_eq!(series(&h), before - 1, "only that occurrence goes");
        h.query_all_by_label_contains("Weekstart").next().expect("another Weekstart").click_secondary();
        h.run();
        h.get_by_label(&calendar::delete_series_menu()).click();
        h.run();
        assert_eq!(series(&h), 0, "the whole series goes");
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
        assert_eq!(window_title(0), "Merel");
        assert_eq!(window_title(3), "Merel (3)");
        let mut h = harness();
        h.run();
        let unread = h.state().engine.session.inbox_unread();
        assert!(unread > 0);
        assert_eq!(h.state().title.as_deref(), Some(format!("Merel ({unread})").as_str()));
        h.get_by_label("Map").click();
        h.run();
        h.get_by_label("Alles gelezen").click();
        h.run();
        assert_eq!(h.state().title.as_deref(), Some("Merel"));
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
        h.get_by_label("Welkom bij Merel").click();
        h.run();
        h.key_press(egui::Key::Delete);
        h.run();
        assert_eq!(h.state().engine.session.message("m0").map(|m| m.folder_id.as_str()), Some("deleted"), "first to the deleted folder");
        h.get_by_label("Verwijderde items").click();
        h.run();
        h.get_by_label("Welkom bij Merel").click();
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
        h.get_by_label(&view::all_categories()).click();
        h.run();
        h.get_by_label("Groene categorie").click();
        h.run();
        assert_eq!(h.state().engine.session.category_filter.as_deref(), Some("Groene categorie"));
        assert!(h.query_all_by_label("Welkom bij Merel").next().is_none(), "only mail with the category is listed");
        h.get_by_label("Planning voor donderdag");
        h.get_by_label("Groene categorie").click();
        h.run();
        h.get_by_label(&view::all_categories()).click();
        h.run();
        assert!(h.state().engine.session.category_filter.is_none());
        h.get_by_label("Ongelezen").click();
        h.run();
        assert!(h.query_all_by_label("Re: Offerte drukwerk").next().is_none(), "read mail is hidden");
        h.get_by_label("Welkom bij Merel");
        h.get_by_label("Met bijlage").click();
        h.run();
        assert!(h.query_all_by_label("Welkom bij Merel").next().is_none());
        h.get_by_label("Planning voor donderdag").click();
        h.run();
        h.get_by_label("Alles").click();
        h.run();
        assert_eq!(h.state().engine.session.visible_messages().len(), 4);

        h.get_by_label("Onder").click();
        h.run();
        assert_eq!(h.state().view.reading_pane, view::ReadingPane::Bottom);
        h.get_by_label("Welkom bij Merel").click();
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
        let in_list = h.get_all_by_label("Planning voor donderdag").count();
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(h.state().message_window);
        // The window is titled with the subject, as the row in the list is.
        assert!(h.get_all_by_label("Planning voor donderdag").count() > in_list);
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(!h.state().message_window && h.get_all_by_label("Planning voor donderdag").count() == in_list);
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

        h.get_by_label("Welkom bij Merel").click();
        h.run();
        h.get_by_label("Klaar").click();
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
        // The Help tab itself shows the version too, next to the button.
        h.get_by_label(help::version_line().as_str());
        h.get_by_label("Over Merel").click();
        h.run();
        assert_eq!(h.query_all_by_label(help::version_line().as_str()).count(), 2);
        h.get_by_label("Kopiëren");
        h.get_by_label("Sluiten").click();
        h.run();
        assert!(h.state().help.is_none());
    }

    #[test]
    fn a_category_is_put_on_a_message_from_the_menu_and_the_list_is_managed() {
        let mut h = harness();
        h.run();
        h.get_by_label("Welkom bij Merel").click();
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
    fn bestand_meldingen_switches_notifications_and_sends_a_trial_one() {
        let mut h = harness();
        h.run();
        h.get_by_label("Bestand").click();
        h.run();
        h.get_by_label(&notifications::page_label()).click();
        h.run();
        assert!(h.query_by_label("Toevoegen en verbinden").is_none(), "the accounts page made way");
        assert!(!h.state().view.notifications.details, "private unless asked");
        h.get_by_label(&notifications::details()).click();
        h.get_by_label(&notifications::badge()).click();
        h.run();
        assert!(h.state().view.notifications.details);
        assert!(!h.state().view.notifications.badge);
        h.get_by_label(&notifications::try_label()).click();
        h.run();
        assert_eq!(h.state().alerts.len(), 1);
        assert_eq!(h.state().alerts[0].title, "Merel");
    }

    #[test]
    fn a_clicked_notification_opens_its_message() {
        let mut h = harness();
        h.state_mut().module = Module::Calendar;
        h.run();
        notifications::open(h.state_mut(), "mail:m1");
        h.run();
        assert_eq!(h.state().module, Module::Mail);
        assert_eq!(h.state().engine.session.selected_message.as_deref(), Some("m1"));
        notifications::open(h.state_mut(), "task:nothing");
        assert_eq!(h.state().module, Module::Tasks);
    }

    #[test]
    fn file_tab_opens_the_backstage_with_the_account_form() {
        let mut h = harness();
        h.run();
        h.get_by_label("Bestand").click();
        h.run();
        assert!(h.query_by_label("Toevoegen en verbinden").is_some());
        h.get_by_label(&backstage::back()).click();
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
