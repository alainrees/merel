//! The chat module: the chats on the left, the open one on the right with its messages and a
//! line to write, and a window for a new chat that picks people from the address book. Chat
//! goes over e-mail, Delta Chat's way, and only end-to-end encrypted; see `merel_engine::chat`.
//! Someone without a key yet gets an invitation by mail from Merel itself, and the message
//! waits until their key comes (`merel_engine::securejoin`).

use chrono::{Local, NaiveDate};
use egui::{Align, FontId, Layout, Margin, RichText, Sense, Ui, vec2};

use merel_engine::SendState;
use merel_engine::chat::{self as engine_chat, Chat};
use merel_engine::securejoin::{INVITE_LINK, Waiting};
use merel_model::Address;

use crate::calendar::to_local;
use crate::helder::{self, Icon};
use crate::i18n::{tr, tr_args};
use crate::{MailApp, Tokens};

/// The search box's widget id, so Ctrl+E can put the cursor there.
pub(crate) const SEARCH_BOX: &str = "chat_search_box";
/// The line to write in: its widget id.
pub(crate) const INPUT: &str = "chat-input";
/// The field for people in a new chat: its widget id.
pub(crate) const PEOPLE: &str = "chat-people";
/// The most suggestions shown under the people field.
const MAX_SUGGESTIONS: usize = 6;

/// Which chats the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Unread,
    Groups,
}

/// What the chat module remembers between frames.
#[derive(Default)]
pub struct State {
    pub selected: Option<String>,
    pub filter: Filter,
    pub query: String,
    /// What is typed in the open chat.
    pub input: String,
    /// The message the next one answers, quoted above it (Signal).
    pub replying: Option<String>,
    /// Files going with the next message (Signal).
    pub files: Vec<merel_model::DraftAttachment>,
    /// One's own message whose text the line to write in changes (Signal).
    pub editing: Option<String>,
    /// The message the window to delete it asks about (Signal).
    pub deleting: Option<String>,
    /// The chat the window to delete it asks about.
    pub deleting_chat: Option<String>,
    /// The voice messages heard and the one playing (Signal).
    pub voice: crate::voice::Player,
    /// A voice message being recorded (Signal).
    pub recording: Option<merel_voice::Recording>,
    pub new: Option<NewChat>,
    /// Why the last message did not go out, or chat could not be turned on.
    pub error: Option<String>,
    /// The message on its way, by outbox id.
    pub sending: Option<u64>,
    /// The invitation link on show, with its account.
    pub invite: Option<(String, String)>,
    /// When the link was last copied (egui time), to say so for a moment.
    pub copied_at: Option<f64>,
    /// An invitation link pasted to accept, and the account it is for.
    pub accept: String,
    pub accept_account: String,
    /// What became of the last invitation accepted.
    pub accept_note: Option<Result<String, String>>,
}

/// The window for a new chat.
#[derive(Default)]
pub struct NewChat {
    /// The account to chat from.
    pub account_id: String,
    /// The people picked, as chips.
    pub people: Vec<Address>,
    /// What is typed after the chips: part of a name or an address.
    pub typed: String,
    /// Outlook's "Groepschat": the chat gets a name and everyone sees everyone.
    pub group: bool,
    pub name: String,
    pub text: String,
    pub error: Option<String>,
}

impl MailApp {
    /// Opens the window for a new chat, from the first account that can chat.
    pub fn new_chat(&mut self) {
        let account_id = self.chat_accounts().into_iter().next();
        self.chat.new = Some(NewChat { account_id: account_id.unwrap_or_default(), ..NewChat::default() });
        self.chat.error = None;
    }

    /// Opens the chat with `person`, as the contacts module offers it: the one already
    /// going, else the window for a new chat with them.
    pub fn chat_with(&mut self, person: Address) {
        self.module = crate::Module::Chat;
        self.new_chat();
        // Their Signal contact, by name; else the name stays typed to look for them.
        let account_id = self.chat.new.as_ref().map(|f| f.account_id.clone()).unwrap_or_default();
        let name = person.display().to_lowercase();
        let known = self.engine.signal_contacts(&account_id).into_iter().map(|(a, _)| a).find(|a| a.display().to_lowercase() == name);
        let existing = known.as_ref().and_then(|k| {
            self.engine.session.chats().into_iter().find(|c| !c.is_group() && c.people.iter().any(|p| p.email().eq_ignore_ascii_case(k.email())))
        });
        if let Some(chat) = existing {
            self.chat.new = None;
            self.open_chat(&chat.id);
            return;
        }
        if let Some(form) = self.chat.new.as_mut() {
            match known {
                Some(a) => form.people.push(a),
                None => form.typed = person.display().to_string(),
            }
        }
    }

    /// Whether any account can carry chat.
    pub(crate) fn can_chat(&self) -> bool {
        !self.chat_accounts().is_empty()
    }

    /// The accounts that can carry chat, by id: Signal's. Chat over mail is gone (Alain
    /// 2026-10-30); what was said that way reads as mail again.
    fn chat_accounts(&self) -> Vec<String> {
        self.engine.session.accounts.iter().filter(|a| merel_engine::signal::is_signal(a)).map(|a| a.id.clone()).collect()
    }

    fn open_chat(&mut self, id: &str) {
        self.chat.replying = None;
        self.stop_editing();
        self.chat.files.clear();
        self.chat.selected = Some(id.to_string());
        self.chat.new = None;
        self.chat.error = None;
        self.engine.open_chat(id);
    }

    /// The chat `id`: one with messages, or one whose first message waits for a key.
    fn chat_by_id(&self, id: &str) -> Option<Chat> {
        self.engine.session.chat(id)
    }

    /// Leaves changing a message; what was typed for it goes.
    fn stop_editing(&mut self) {
        if self.chat.editing.take().is_some() {
            self.chat.input.clear();
        }
    }

    /// Sends what is typed in the open chat: a new message, or the new text of the one being
    /// changed.
    fn send_in_chat(&mut self) {
        let Some(chat) = self.chat.selected.as_deref().and_then(|id| self.chat_by_id(id)) else { return };
        let text = self.chat.input.trim().to_string();
        if let Some(id) = self.chat.editing.clone() {
            match self.engine.signal_edit(&id, &text) {
                Ok(()) => {
                    self.chat.editing = None;
                    self.chat.input.clear();
                    self.chat.error = None;
                }
                Err(e) => self.chat.error = Some(e),
            }
            return;
        }
        if text.is_empty() && self.chat.files.is_empty() {
            return;
        }
        let mut draft = self.engine.session.chat_reply(&chat, &text);
        draft.attachments = self.chat.files.clone();
        if let (Some(id), Some(tag)) = (self.chat.replying.as_deref(), draft.chat.as_mut()) {
            tag.quote = self.engine.session.chat_quote(id);
        }
        match self.engine.send_or_invite(&chat.account_id, draft) {
            Ok(id) => {
                self.chat.sending = id;
                self.chat.input.clear();
                self.chat.replying = None;
                self.chat.files.clear();
                self.chat.error = None;
            }
            Err(e) => self.chat.error = Some(e),
        }
    }

    /// Stops recording and sends what was said as a voice message, as Signal does: MP3,
    /// which every Signal app plays. Under half a second is a slip and goes nowhere.
    fn send_voice(&mut self) {
        let Some(recording) = self.chat.recording.take() else { return };
        let clip = recording.finish();
        let Some(chat) = self.chat.selected.as_deref().and_then(|id| self.chat_by_id(id)) else { return };
        if clip.duration() < std::time::Duration::from_millis(500) {
            return;
        }
        let bytes = match merel_voice::mp3(&clip) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.chat.error = Some(e);
                return;
            }
        };
        let mut draft = self.engine.session.chat_reply(&chat, "");
        draft.attachments = vec![merel_model::DraftAttachment::new(&format!("{}.mp3", tr("chat-voice-file")), bytes)];
        if let Some(tag) = draft.chat.as_mut() {
            tag.voice = true;
            if let Some(id) = self.chat.replying.as_deref() {
                tag.quote = self.engine.session.chat_quote(id);
            }
        }
        match self.engine.send_or_invite(&chat.account_id, draft) {
            Ok(id) => {
                self.chat.sending = id;
                self.chat.replying = None;
                self.chat.error = None;
            }
            Err(e) => self.chat.error = Some(e),
        }
    }

    /// Starts the chat the window describes: sends the first message, or holds it while an
    /// invitation goes to whoever has no key yet.
    fn start_chat(&mut self) {
        let contacts = self.chat.new.as_ref().map(|f| self.engine.signal_contacts(&f.account_id)).unwrap_or_default();
        let Some(form) = self.chat.new.as_mut() else { return };
        if !commit_typed(form, &contacts) {
            return;
        }
        if form.people.len() != 1 {
            form.error = Some(tr(if form.people.is_empty() { "chat-pick-someone" } else { "signal-one-person" }));
            return;
        }
        let draft = match engine_chat::new_chat(&form.people, "", &form.text) {
            Ok(d) => d,
            Err(e) => {
                form.error = Some(e);
                return;
            }
        };
        let account_id = form.account_id.clone();
        let id = engine_chat::person_chat_id(&account_id, draft.to.first().map(Address::email).unwrap_or(""));
        match self.engine.send_or_invite(&account_id, draft) {
            Ok(sent) => {
                self.chat.sending = sent;
                self.chat.new = None;
                self.chat.selected = Some(id);
            }
            Err(e) => {
                if let Some(form) = self.chat.new.as_mut() {
                    form.error = Some(e);
                }
            }
        }
    }
}

/// Turns what is typed in the people field into a chip, when it names one of the Signal
/// `contacts` (by name or number); false (with the reason in the form) when it does not.
fn commit_typed(form: &mut NewChat, contacts: &[(Address, String)]) -> bool {
    let typed = form.typed.trim().trim_end_matches([',', ';']).trim().to_lowercase();
    if typed.is_empty() {
        return true;
    }
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let found = contacts.iter().find(|(a, number)| {
        a.display().to_lowercase() == typed || (!number.is_empty() && digits(number) == digits(&typed) && !digits(&typed).is_empty())
    });
    match found {
        Some((a, _)) => {
            add_person(form, a.clone());
            form.typed.clear();
            true
        }
        None => {
            form.error = Some(tr("chat-not-a-contact"));
            false
        }
    }
}

fn add_person(form: &mut NewChat, person: Address) {
    if !form.people.iter().any(|p| p.email().eq_ignore_ascii_case(person.email())) {
        form.people.push(person);
    }
    form.error = None;
}

/// The messages of `chat` that wait for a key.
fn waiting_in<'a>(app: &'a MailApp, chat: &Chat) -> Vec<&'a Waiting> {
    app.engine.waiting().iter().filter(|w| w.chat_id() == chat.id).collect()
}

/// The chats the list shows, by the filter and the search box.
fn listed(app: &MailApp) -> Vec<Chat> {
    let needle = app.chat.query.trim().to_lowercase();
    let signal = app.chat_accounts();
    app.engine
        .session
        .chats()
        .into_iter()
        .filter(|c| signal.contains(&c.account_id))
        .filter(|c| match app.chat.filter {
            Filter::All => true,
            Filter::Unread => c.unread > 0,
            Filter::Groups => c.is_group(),
        })
        .filter(|c| {
            needle.is_empty()
                || c.title.to_lowercase().contains(&needle)
                || c.people.iter().any(|p| p.email().to_lowercase().contains(&needle) || p.display().to_lowercase().contains(&needle))
        })
        .collect()
}

/// "14:05" today, else "21-10".
fn when(ts: i64) -> String {
    let at = to_local(ts);
    if at.date() == Local::now().date_naive() { at.format("%H:%M").to_string() } else { at.format("%d-%m").to_string() }
}

/// "Vandaag", "Gisteren" or the date, over the messages of a day.
fn day_label(d: NaiveDate) -> String {
    let today = Local::now().date_naive();
    if d == today {
        tr("chat-today")
    } else if today.pred_opt() == Some(d) {
        tr("chat-yesterday")
    } else {
        crate::dates::day(crate::calendar::to_timestamp(d.and_hms_opt(12, 0, 0).unwrap_or_default()).unwrap_or(0))
    }
}

/// The invitation link in a message's text, when it carries one.
pub(crate) fn invite_in(text: &str) -> Option<&str> {
    let at = text.find(INVITE_LINK)?;
    let link = &text[at..];
    Some(link.split(char::is_whitespace).next().unwrap_or(link))
}

pub fn show(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    list_pane(app, ui, t);
    delete_chat_window(app, ui.ctx(), t);
    let open = app.chat.selected.as_deref().and_then(|id| app.chat_by_id(id));
    match open {
        Some(chat) => {
            let frame = helder::card_frame(t).inner_margin(Margin::same(20)).outer_margin(Margin { left: 6, right: 12, top: 0, bottom: 12 });
            egui::CentralPanel::default().frame(frame).show(ui, |ui| conversation(app, ui, t, &chat));
        }
        None => {
            // The start page is a column of cards on the window's grey ground.
            let frame = egui::Frame::new().outer_margin(Margin { left: 6, right: 12, top: 0, bottom: 12 });
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| start_page(app, ui, t));
            });
        }
    }
}

/// The new-chat window, wherever the person is in Merel.
pub fn windows(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    if app.chat.new.is_none() {
        return;
    }
    let mut open = true;
    helder::item_window(ctx, t, "new-chat-window", &tr("chat-new"), vec2(720.0, 560.0), &mut open, |ui| new_chat_form(app, ui, t));
    if !open {
        app.chat.new = None;
    }
}

fn list_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("chat-list")
        .default_size(340.0)
        .resizable(true)
        .show_separator_line(false)
        .frame(helder::card_frame(t).inner_margin(Margin::same(12)).outer_margin(Margin { left: 0, right: 6, top: 0, bottom: 12 }))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("chat-all")).size(18.0).color(t.text));
                helder::trailing(ui, |ui| {
                    if helder::quiet(ui, t, |ui| helder::icon_button(ui, t, Icon::MessagesSquare, &tr("chat-new"))).clicked() {
                        app.new_chat();
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 40.0).max(60.0);
                helder::search_field(ui, t, &mut app.chat.query, egui::Id::new(SEARCH_BOX), &tr("chat-search"), width);
                if !app.chat.query.is_empty() && helder::quiet(ui, t, |ui| helder::icon_button(ui, t, Icon::X, &crate::search_clear())).clicked() {
                    app.chat.query.clear();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for (key, f) in [("chat-filter-all", Filter::All), ("chat-filter-unread", Filter::Unread), ("chat-filter-groups", Filter::Groups)] {
                    if helder::chip(ui, t, &tr(key), app.chat.filter == f).clicked() {
                        app.chat.filter = f;
                    }
                }
            });
            ui.add_space(8.0);
            let list = listed(app);
            if list.is_empty() {
                let text = if app.chat.query.trim().is_empty() { tr("chat-none") } else { tr("chat-not-found") };
                ui.label(RichText::new(text).color(t.text_dim));
            }
            let mut pick = None;
            let mut delete = None;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for chat in &list {
                    let row = chat_row(app, ui, t, chat);
                    if row.clicked() {
                        pick = Some(chat.id.clone());
                    }
                    row.context_menu(|ui| {
                        if ui.add(egui::Button::image_and_text(Icon::Trash.image(16.0, t.text), tr("chat-delete-chat"))).clicked() {
                            delete = Some(chat.id.clone());
                            ui.close();
                        }
                    });
                }
            });
            if let Some(id) = pick {
                app.open_chat(&id);
            }
            if delete.is_some() {
                app.chat.deleting_chat = delete;
            }
        });
}

/// The avatar of a chat: the group's, or the other person's.
fn chat_avatar(p: &egui::Painter, center: egui::Pos2, radius: f32, chat: &Chat) {
    let first = chat.people.first();
    let (name, mail) = if chat.is_group() {
        (chat.title.as_str(), chat.id.as_str())
    } else {
        (first.map(Address::display).unwrap_or(""), first.map(Address::email).unwrap_or(""))
    };
    helder::avatar(p, center, radius, name, mail);
}

/// One chat in the list: avatar, name, when, the last message, and how many are unread.
fn chat_row(app: &MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) -> egui::Response {
    let on = app.chat.selected.as_deref() == Some(chat.id.as_str());
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
    let label = if chat.unread > 0 {
        tr_args("chat-unread-label", &[("title", chat.title.as_str().into()), ("count", chat.unread.into())])
    } else {
        chat.title.clone()
    };
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, &label));
    let p = ui.painter();
    if on || r.hovered() {
        p.rect_filled(rect, 8, if on { t.checked } else { t.hover });
    }
    if on {
        // The open chat has the accent's bar on its left edge, as in the design.
        p.rect_filled(egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 8.0), vec2(3.0, rect.height() - 16.0)), 1.5, t.accent);
    }
    chat_avatar(p, egui::pos2(rect.left() + 26.0, rect.center().y), 18.0, chat);
    let x = rect.left() + 54.0;
    let right = rect.right() - 8.0;
    let stamp = when(chat.last);
    let stamp_w = p.layout_no_wrap(stamp.clone(), FontId::proportional(12.0), t.text_dim).size().x;
    p.text(egui::pos2(right, rect.top() + 11.0), egui::Align2::RIGHT_TOP, stamp, FontId::proportional(12.0), t.text_dim);
    let unread = chat.unread > 0;
    let name_color = if unread { t.accent_text } else { t.text };
    let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(right - stamp_w - 8.0, rect.bottom()));
    p.with_clip_rect(clip).text(egui::pos2(x, rect.top() + 10.0), egui::Align2::LEFT_TOP, &chat.title, FontId::proportional(14.5), name_color);
    let preview = match chat.messages.last() {
        Some(last) => {
            // Over Signal the subject is the text's start, there before the message is opened.
            let signal =
                app.engine.is_signal_account(&chat.account_id).then(|| app.engine.session.message(last).map(|m| m.subject.clone())).flatten();
            let text =
                app.engine.session.chat_text(last).map(|s| s.replace('\n', " ")).or(signal).unwrap_or_else(|| format!("🔒 {}", tr("chat-encrypted")));
            let text = if invite_in(&text).is_some() { tr("chat-invitation") } else { text };
            if app.engine.session.chat_is_mine(last) { tr_args("chat-mine", &[("text", text.as_str().into())]) } else { text }
        }
        None => tr_args("chat-waiting-short", &[("name", chat.title.as_str().into())]),
    };
    let badge_w = if unread { 26.0 } else { 0.0 };
    let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(right - badge_w, rect.bottom()));
    let preview_color = if unread { t.text } else { t.text_dim };
    p.with_clip_rect(clip).text(egui::pos2(x, rect.top() + 31.0), egui::Align2::LEFT_TOP, preview, FontId::proportional(13.0), preview_color);
    if unread {
        let c = egui::pos2(right - 10.0, rect.top() + 39.0);
        p.circle_filled(c, 9.0, t.accent);
        p.text(c, egui::Align2::CENTER_CENTER, chat.unread.min(99).to_string(), FontId::proportional(11.0), t.on_accent);
    }
    r
}

/// The open chat: its name, the messages oldest first with a line per day, and the line to
/// write.
fn conversation(app: &mut MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) {
    let signal_chat = app.engine.is_signal_account(&chat.account_id);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        chat_avatar(ui.painter(), rect.center(), 20.0, chat);
        ui.vertical(|ui| {
            ui.label(RichText::new(&chat.title).size(17.0).color(t.text));
            let first = chat.people.first();
            let signal = app.engine.is_signal_account(&chat.account_id);
            // Signal's addresses mean nothing to anyone: the channel says enough.
            let people = if signal { tr("signal-via") } else { chat.people.iter().map(Address::email).collect::<Vec<_>>().join(", ") };
            let verified = !chat.is_group() && first.is_some_and(|p| app.engine.is_verified(p.email()));
            ui.horizontal(|ui| {
                ui.add(Icon::Lock.image(14.0, t.text_dim));
                let mut line = format!("{} · {people}", tr("chat-encrypted"));
                if verified {
                    line = format!("{line} · {}", tr("chat-verified"));
                }
                ui.label(RichText::new(line).size(12.5).color(t.text_dim));
            });
        });
        helder::right(ui, |ui| {
            if helder::secondary(ui, t, |ui| helder::tool_button(ui, t, Icon::Trash, &tr("delete")))
                .on_hover_text(tr("chat-delete-chat-help"))
                .clicked()
            {
                app.chat.deleting_chat = Some(chat.id.clone());
            }
            call_button(app, ui, t, chat);
        });
    });
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, t.card_border));
    ui.add_space(4.0);
    // The line to write sits at the bottom; the messages fill what is above it.
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        composer(app, ui, t, chat);
        picked_files(app, ui, t);
        replying_strip(app, ui, t);
        editing_strip(app, ui, t);
        delete_window(app, ui.ctx(), t);
        if signal_chat {
            typing_line(app, ui, t, chat);
        }
        if let Some(e) = &app.chat.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        if let Some(SendState::Failed(e)) = app.chat.sending.and_then(|id| app.engine.session.outbox.get(&id)) {
            ui.label(RichText::new(tr_args("chat-not-sent", &[("reason", e.as_str().into())])).color(t.flag));
        }
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            let mut react = None;
            let mut reply = None;
            let mut picture = None;
            let mut save = None;
            let mut listen = None;
            let mut edit = None;
            let mut delete = None;
            egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
                let mut day = None;
                let mut previous: Option<String> = None;
                for id in &chat.messages {
                    let Some(m) = app.engine.session.message(id) else { continue };
                    let date = to_local(m.date).date();
                    if day != Some(date) {
                        day = Some(date);
                        previous = None;
                        day_line(ui, t, &day_label(date));
                    }
                    let from = m.from.email().to_lowercase();
                    let first_of_run = previous.as_deref() != Some(from.as_str());
                    previous = Some(from);
                    match bubble(app, ui, t, chat, id, first_of_run) {
                        Some(Clicked::React(emoji)) => react = Some((id.clone(), emoji)),
                        Some(Clicked::Reply) => reply = Some(id.clone()),
                        Some(Clicked::Picture(n)) => picture = Some((id.clone(), n)),
                        Some(Clicked::Save(n)) => save = Some((id.clone(), n)),
                        Some(Clicked::Voice(n, ask)) => listen = Some((id.clone(), n, ask)),
                        Some(Clicked::Edit) => edit = Some(id.clone()),
                        Some(Clicked::Delete) => delete = Some(id.clone()),
                        None => {}
                    }
                }
                let waiting = waiting_in(app, chat);
                if !waiting.is_empty() {
                    if day != Some(Local::now().date_naive()) {
                        day_line(ui, t, &tr("chat-today"));
                    }
                    for w in waiting {
                        waiting_bubble(ui, t, chat, w);
                    }
                }
            });
            let file = |app: &MailApp, (id, n): &(String, usize)| app.engine.session.bodies.get(id).and_then(|b| b.attachments.get(*n).cloned());
            if let Some(a) = picture.as_ref().and_then(|p| file(app, p)) {
                let uri = crate::attachments::preview_uri(&picture.as_ref().map(|p| p.0.clone()).unwrap_or_default(), picture.map_or(0, |p| p.1));
                app.preview = Some(crate::attachments::Preview { attachment: a, uri });
            }
            if let Some(a) = save.as_ref().and_then(|p| file(app, p)) {
                app.save_attachment(&a);
            }
            if let Some((id, n, ask)) = listen
                && let Some(a) = file(app, &(id.clone(), n))
            {
                app.chat.error = app.chat.voice.ask(&id, n, &a, ask).err();
            }
            app.chat.voice.tick(ui.ctx());
            if let Some(id) = edit {
                app.chat.replying = None;
                app.chat.files.clear();
                app.chat.input = app.engine.session.chat_text(&id).unwrap_or_default();
                app.chat.editing = Some(id);
                ui.memory_mut(|m| m.request_focus(egui::Id::new(INPUT)));
            }
            if delete.is_some() {
                app.chat.deleting = delete;
            }
            if let Some(id) = reply {
                app.stop_editing();
                app.chat.replying = Some(id);
                ui.memory_mut(|m| m.request_focus(egui::Id::new(INPUT)));
            }
            if let Some((id, emoji)) = react {
                app.chat.error = app.engine.react(&id, &emoji).err();
            }
        });
    });
}

/// Bellen, or Deelnemen while the others are in a meeting of this chat, or Terug naar het
/// gesprek while this computer is in one.
fn call_button(app: &mut MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) {
    // A Signal chat with one person is called over Signal, when this build of its program can.
    if app.engine.is_signal_account(&chat.account_id) {
        if let ([person], false) = (chat.people.as_slice(), chat.is_group())
            && crate::signal_call::can_call(app, &chat.account_id)
            && helder::secondary(ui, t, |ui| helder::tool_button(ui, t, Icon::Phone, &tr("call-start")))
                .on_hover_text(tr("signal-call-start-help"))
                .clicked()
        {
            let (account, with, name) = (chat.account_id.clone(), person.email().to_string(), chat.title.clone());
            crate::signal_call::start(app, &account, &with, &name);
        }
        return;
    }
    if let Some(room) = crate::room::joined_in(app, &chat.id).map(|c| c.room.clone()) {
        if helder::secondary(ui, t, |ui| helder::tool_button(ui, t, Icon::Phone, &tr("call-back"))).clicked() {
            app.room.open = Some(room);
        }
    } else if let Some(room) = crate::room::ringing_in(app, &chat.id).map(|c| c.room.clone()) {
        if helder::primary_button(ui, t, Icon::Phone, &tr("meet-join")).clicked() {
            crate::room::join(app, &room);
        }
    } else if helder::secondary(ui, t, |ui| helder::tool_button(ui, t, Icon::Phone, &tr("call-start"))).on_hover_text(tr("call-start-help")).clicked()
    {
        crate::room::start(app, &chat.id);
    }
}

/// "Vandaag" between two thin lines.
fn day_line(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let p = ui.painter();
    let w = p.layout_no_wrap(text.to_string(), FontId::proportional(12.0), t.text_dim).size().x;
    let c = rect.center();
    p.hline(rect.left()..=c.x - w / 2.0 - 10.0, c.y, egui::Stroke::new(1.0, t.card_border));
    p.hline(c.x + w / 2.0 + 10.0..=rect.right(), c.y, egui::Stroke::new(1.0, t.card_border));
    p.text(c, egui::Align2::CENTER_CENTER, text, FontId::proportional(12.0), t.text_dim);
    ui.add_space(6.0);
}

/// What was clicked on a message.
enum Clicked {
    /// React with this emoji, or take it back when it was the account's own.
    React(String),
    /// Answer this message.
    Reply,
    /// Open the picture that is attachment `n` of the message.
    Picture(usize),
    /// Save attachment `n` of the message under Downloads.
    Save(usize),
    /// Play, pause or jump in the voice message that is attachment `n`.
    Voice(usize, crate::voice::Ask),
    /// Change the text of one's own message.
    Edit,
    /// Ask how to delete the message.
    Delete,
}

/// The reactions Signal offers first, in its order.
const QUICK_REACTIONS: [&str; 6] = ["❤️", "👍", "👎", "😂", "😮", "😢"];

/// One message: on the right in the accent's tint when it is one's own, else on the left
/// with the sender's avatar and name over the first of a run. An invitation shows as one,
/// with a button to accept it. In a Signal chat the reactions show under it, and a smiley
/// beside it (on hover, or right-click the message) reacts.
fn bubble(app: &MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat, id: &str, first_of_run: bool) -> Option<Clicked> {
    let session = &app.engine.session;
    let m = session.message(id)?;
    let mine = session.chat_is_mine(id);
    let text = session.chat_text(id).unwrap_or_else(|| tr("chat-loading"));
    let invite = invite_in(&text).map(str::to_string);
    let ring = m.chat.as_ref().and_then(|c| c.call.as_ref()).is_some_and(|c| c.step == merel_model::CallStep::Ring);
    let max = (ui.available_width() * 0.7).min(480.0);
    let mut accept = None;
    let signal = app.engine.is_signal_account(&chat.account_id);
    let deleted = m.chat.as_ref().is_some_and(|c| c.deleted);
    let edited = m.chat.as_ref().is_some_and(|c| c.edited);
    let reacts = signal && invite.is_none() && !ring && !deleted;
    let can_change = reacts && app.engine.signal_can_change(id);
    let reactions = m.chat.as_ref().map(|c| c.reactions.clone()).unwrap_or_default();
    // Whether the pointer was over this message last frame: then the smiley shows.
    let hover_id = egui::Id::new(("chat-hover", id));
    let hovered = ui.data(|d| d.get_temp::<bool>(hover_id)).unwrap_or(false);
    // Your own messages line up on the right, the others' on the left, as in any chat.
    let layout = Layout::top_down(if mine { Align::Max } else { Align::Min });
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 0.0), layout, |ui| {
        if first_of_run {
            ui.add_space(4.0);
            let who = if mine { when(m.date) } else { format!("{} · {}", m.from.display(), when(m.date)) };
            ui.horizontal(|ui| {
                if !mine {
                    ui.add_space(40.0);
                }
                ui.label(RichText::new(who).size(12.0).color(t.text_dim));
            });
        }
        // The right-click menu takes the whole row. It goes under the buttons, links and files
        // in it, so they still take their own clicks: egui gives a click to the widget drawn
        // last, so it is laid down first, where the row was last frame.
        let rect_id = egui::Id::new(("chat-bubble-rect", id));
        let menu = (signal && (reacts || deleted))
            .then(|| ui.data(|d| d.get_temp::<egui::Rect>(rect_id)))
            .flatten()
            .map(|rect| ui.interact(rect, egui::Id::new(("chat-bubble", id)), Sense::click()));
        // The text takes its own clicks too (to select it); a right click there gives the menu.
        let mut text_clicks = None;
        let row = ui.horizontal_top(|ui| {
            if mine && reacts {
                react_button(ui, t, id, hovered, &mut accept);
                reply_button(ui, t, hovered, &mut accept);
            }
            if !mine {
                let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                if first_of_run {
                    helder::avatar(ui.painter(), rect.center(), 16.0, m.from.display(), m.from.email());
                }
                ui.add_space(4.0);
            }
            egui::Frame::new().fill(if mine { t.checked } else { t.window }).corner_radius(12).inner_margin(Margin::symmetric(12, 8)).show(
                ui,
                |ui| {
                    ui.set_max_width(max);
                    let quote = m.chat.as_ref().and_then(|c| c.quote.as_ref());
                    let mut content = |ui: &mut Ui| match &invite {
                        Some(_) => {
                            ui.horizontal(|ui| {
                                ui.add(Icon::Link.image(16.0, t.text));
                                let line =
                                    if mine { tr("chat-invite-sent") } else { tr_args("chat-invite-received", &[("name", m.from.display().into())]) };
                                ui.add(egui::Label::new(RichText::new(line).size(14.0).color(t.text)).wrap());
                            });
                        }
                        None if ring => {
                            ui.horizontal(|ui| {
                                ui.add(Icon::Phone.image(16.0, t.text));
                                let line =
                                    if mine { tr("call-ring-mine") } else { tr_args("call-ring-theirs", &[("name", m.from.display().into())]) };
                                ui.add(egui::Label::new(RichText::new(line).size(14.0).color(t.text)).wrap());
                            });
                        }
                        // As Signal leaves it: a quiet line where the message was.
                        None if deleted => {
                            ui.horizontal(|ui| {
                                ui.add(Icon::Trash.image(14.0, t.text_dim));
                                ui.label(RichText::new(tr("signal-deleted")).size(14.0).italics().color(t.text_dim));
                            });
                        }
                        None => {
                            let files = session.bodies.get(id).map(|b| b.attachments.as_slice()).unwrap_or_default();
                            if !text.trim().is_empty() || files.is_empty() {
                                text_clicks = Some(crate::emoji::label(ui, &text, 14.0, t.text));
                            }
                            let voice = m.chat.as_ref().is_some_and(|c| c.voice);
                            files_in_bubble(ui, t, &app.chat.voice, id, files, voice, &mut accept);
                        }
                    };
                    // A quote goes above the text and files under it; without either the text
                    // keeps the bubble narrow.
                    let has_files = session.bodies.get(id).is_some_and(|b| !b.attachments.is_empty());
                    // In the bubble's corner, as Signal shows it.
                    let ticks = mine && reacts;
                    let delivery = m.chat.as_ref().and_then(|c| c.delivery);
                    let corner = |ui: &mut Ui| {
                        if ticks {
                            delivery_ticks(ui, t, delivery);
                        }
                        if edited && !deleted {
                            ui.label(RichText::new(tr("chat-edited")).size(12.0).color(t.text_dim));
                        }
                    };
                    if quote.is_some() || has_files {
                        ui.vertical(|ui| {
                            if let Some(q) = quote.filter(|_| !deleted) {
                                quote_block(ui, t, session, q, mine);
                            }
                            content(ui);
                            if ticks || edited {
                                ui.with_layout(Layout::right_to_left(Align::Min), corner);
                            }
                        });
                    } else if mine {
                        // One's own row runs right to left: what comes first ends up after the
                        // text.
                        corner(ui);
                        content(ui);
                    } else {
                        content(ui);
                        if edited && !deleted {
                            ui.label(RichText::new(tr("chat-edited")).size(12.0).color(t.text_dim));
                        }
                    }
                },
            );
            if !mine && reacts {
                reply_button(ui, t, hovered, &mut accept);
                react_button(ui, t, id, hovered, &mut accept);
            }
        });
        if signal && (reacts || deleted) {
            let over = ui.rect_contains_pointer(row.response.rect);
            ui.data_mut(|d| {
                d.insert_temp(hover_id, over);
                d.insert_temp(rect_id, row.response.rect);
            });
            if menu.is_none() {
                ui.ctx().request_repaint();
            }
        }
        if let Some(menu) = menu {
            let menu = match text_clicks {
                Some(text) => menu.union(text),
                None => menu,
            };
            menu.context_menu(|ui| {
                if reacts && ui.add(egui::Button::image_and_text(Icon::Reply.image(16.0, t.text), tr("chat-reply"))).clicked() {
                    accept = Some(Clicked::Reply);
                    ui.close();
                }
                if can_change && ui.add(egui::Button::image_and_text(Icon::Pencil.image(16.0, t.text), tr("chat-edit"))).clicked() {
                    accept = Some(Clicked::Edit);
                    ui.close();
                }
                if ui.add(egui::Button::image_and_text(Icon::Trash.image(16.0, t.text), tr("chat-delete"))).clicked() {
                    accept = Some(Clicked::Delete);
                    ui.close();
                }
                if reacts {
                    ui.separator();
                    reaction_menu(ui, &mut accept);
                }
            });
        }
        if reacts && !reactions.is_empty() {
            ui.horizontal(|ui| {
                if !mine {
                    ui.add_space(40.0);
                }
                reaction_chips(ui, t, &reactions, &mut accept);
            });
        }
    });
    ui.add_space(4.0);
    accept
}

/// The files of a message: pictures small, to open big with a click, as in Signal; other
/// files as a line with their name and size, to save with a click.
fn files_in_bubble(
    ui: &mut Ui,
    t: &Tokens,
    player: &crate::voice::Player,
    id: &str,
    files: &[merel_mime::Attachment],
    voice: bool,
    clicked: &mut Option<Clicked>,
) {
    for (n, a) in files.iter().enumerate() {
        if voice && a.mime.starts_with("audio/") {
            if let Some(ask) = crate::voice::row(ui, t, player, id, n, a) {
                *clicked = Some(Clicked::Voice(n, ask));
            }
        } else if crate::attachments::previewable(a) {
            // Sized from the picture's own header: the bubble's row has no height to fit it in.
            let (w, h) = image::ImageReader::new(std::io::Cursor::new(&a.bytes[..]))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok())
                .unwrap_or((260, 260));
            let scale = (260.0 / w.max(1) as f32).min(260.0 / h.max(1) as f32).min(1.0);
            let image = egui::Image::from_bytes(crate::attachments::preview_uri(id, n), egui::load::Bytes::Shared(a.bytes.clone()))
                .fit_to_exact_size(vec2(w as f32 * scale, h as f32 * scale))
                .corner_radius(8)
                .sense(Sense::click());
            let r = ui.add(image);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, crate::attachments::preview_label(&a.name)));
            if r.on_hover_text(crate::attachments::preview_label(&a.name)).clicked() {
                *clicked = Some(Clicked::Picture(n));
            }
        } else {
            let label = format!("{}  ({})", a.name, crate::size_text(a.size));
            let r = ui.add(egui::Button::image_and_text(Icon::Paperclip.image(14.0, t.text), RichText::new(label).size(13.0).color(t.text)));
            if r.on_hover_text(tr("attachment-save-downloads")).clicked() {
                *clicked = Some(Clicked::Save(n));
            }
        }
    }
}

/// The files picked to go with the next message, each with a cross to leave it out.
fn picked_files(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    if app.chat.files.is_empty() {
        return;
    }
    let mut drop = None;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (n, f) in app.chat.files.iter().enumerate() {
            egui::Frame::new()
                .fill(t.card)
                .stroke(egui::Stroke::new(1.0, t.card_border))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(Icon::Paperclip.image(14.0, t.text_dim));
                        ui.label(RichText::new(format!("{}  ({})", f.name, crate::size_text(f.bytes.len()))).size(12.5).color(t.text));
                        let r = ui.add(egui::Button::image(Icon::X.image(12.0, t.text_dim)).frame(false));
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                tr_args("chat-file-remove", &[("name", f.name.as_str().into())]),
                            )
                        });
                        if r.clicked() {
                            drop = Some(n);
                        }
                    });
                });
        }
    });
    if let Some(n) = drop {
        app.chat.files.remove(n);
    }
}

/// Most bytes of files one Signal message may carry.
const MAX_CHAT_FILES: usize = 100 * 1024 * 1024;

/// Adds the files at `paths` to the next message, within Signal's limit.
fn add_chat_files(app: &mut MailApp, paths: Vec<std::path::PathBuf>) {
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("bestand").to_string();
        let room = MAX_CHAT_FILES.saturating_sub(app.chat.files.iter().map(|f| f.bytes.len()).sum());
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= room => app.chat.files.push(merel_model::DraftAttachment::new(&name, bytes)),
            Ok(_) => app.chat.error = Some(tr("signal-too-big")),
            Err(_) => app.chat.error = Some(tr_args("attach-unreadable", &[("name", name.as_str().into())])),
        }
    }
}

/// In one's own message, as Signal shows it: one tick once sent, two once it arrived,
/// two in the accent once read; the words on hover.
fn delivery_ticks(ui: &mut Ui, t: &Tokens, delivery: Option<merel_model::Delivery>) {
    let (ticks, colour, words) = match delivery {
        None => ("✓", t.text_dim, tr("chat-sent")),
        Some(merel_model::Delivery::Delivered) => ("✓✓", t.text_dim, tr("chat-delivered")),
        Some(merel_model::Delivery::Read) => ("✓✓", t.accent, tr("chat-read")),
    };
    let r = ui.label(RichText::new(ticks).size(12.0).color(colour));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, words.clone()));
    r.on_hover_text(words);
}

/// "Sanne is aan het typen…" above the line to write in, as Signal shows it.
fn typing_line(app: &MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) {
    if let Some(name) = app.engine.signal_typing(chat) {
        ui.add_space(2.0);
        ui.label(RichText::new(tr_args("chat-typing", &[("name", name.as_str().into())])).size(12.5).italics().color(t.text_dim));
    }
    // It ends by itself after a while, so look again.
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
}

/// The arrow beside a message that answers it, quoting it; shown like the smiley.
fn reply_button(ui: &mut Ui, t: &Tokens, hovered: bool, clicked: &mut Option<Clicked>) {
    let size = vec2(28.0, 28.0);
    if !hovered {
        ui.allocate_exact_size(size, Sense::hover());
        return;
    }
    let r = ui.add(egui::Button::image(Icon::Reply.image(16.0, t.text_dim)).frame(false).min_size(size));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("chat-reply")));
    if r.on_hover_text(tr("chat-reply")).clicked() {
        *clicked = Some(Clicked::Reply);
    }
}

/// The message an answer quotes, inside the answer's bubble: a bar in the accent, the
/// writer's name and the first lines of the text, as Signal shows it.
fn quote_block(ui: &mut Ui, t: &Tokens, session: &merel_engine::Session, q: &merel_model::Quoted, mine: bool) {
    // "Jij" for one's own message, as Signal says it.
    let own = session.messages.iter().find(|m| m.message_id.as_deref() == Some(q.message_id.as_str())).is_some_and(|m| session.chat_is_mine(&m.id));
    let name = if own { tr("chat-you") } else { q.name.clone() };
    let fill = if mine { t.window.gamma_multiply(0.6) } else { t.checked.gamma_multiply(0.6) };
    let frame = egui::Frame::new().fill(fill).corner_radius(6).inner_margin(Margin { left: 10, right: 8, top: 4, bottom: 4 }).show(ui, |ui| {
        ui.set_min_width(120.0);
        ui.label(RichText::new(name).size(12.5).strong().color(t.text));
        let text: String = q.text.lines().take(2).collect::<Vec<_>>().join(" ");
        ui.add(egui::Label::new(RichText::new(merel_mime::preview(&text, 140)).size(12.5).color(t.text_dim)).wrap());
    });
    let r = frame.response.rect;
    ui.painter().rect_filled(egui::Rect::from_min_max(r.min, egui::pos2(r.min.x + 3.0, r.max.y)), 2.0, t.accent);
    ui.add_space(4.0);
}

/// Above the line to write in while changing a message: what it said, and a cross to stop.
fn editing_strip(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(id) = app.chat.editing.clone() else { return };
    let mut stop = false;
    ui.add_space(4.0);
    egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(8).inner_margin(Margin::symmetric(10, 6)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                ui.add(Icon::Pencil.image(16.0, t.accent));
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr("chat-editing")).size(12.5).strong().color(t.text));
                    let was = app.engine.session.chat_text(&id).unwrap_or_default();
                    ui.label(RichText::new(merel_mime::preview(&was, 120)).size(12.5).color(t.text_dim));
                });
                helder::right(ui, |ui| {
                    let r = ui.add(egui::Button::image(Icon::X.image(14.0, t.text_dim)).frame(false));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("chat-edit-stop")));
                    stop = r.on_hover_text(tr("chat-edit-stop")).clicked();
                });
            });
        },
    );
    if stop || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.stop_editing();
    }
}

/// The window that asks how to delete a Signal message, as Signal does: only here, or for
/// everyone (one's own, within a day).
fn delete_window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(id) = app.chat.deleting.clone() else { return };
    let everyone = app.engine.signal_can_change(&id);
    let mut choice = None;
    let closed = helder::dialog(ctx, t, &tr("chat-delete-title"), 420.0, |ui| {
        let line = if everyone { tr("chat-delete-ask-everyone") } else { tr("chat-delete-ask") };
        ui.label(RichText::new(line).size(14.0).color(t.text));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if everyone && ui.add(helder::primary_text(t, &tr("chat-delete-everyone"))).clicked() {
                choice = Some(true);
            }
            if helder::secondary(ui, t, |ui| ui.button(tr("chat-delete-me"))).clicked() {
                choice = Some(false);
            }
            if helder::secondary(ui, t, |ui| ui.button(tr("cancel"))).clicked() {
                app.chat.deleting = None;
            }
        });
    });
    if closed {
        app.chat.deleting = None;
    }
    if let Some(everyone) = choice {
        app.chat.deleting = None;
        if app.chat.editing.as_deref() == Some(id.as_str()) {
            app.stop_editing();
        }
        app.chat.error = app.engine.signal_delete(&id, everyone).err();
    }
}

/// The window that asks before a whole chat goes, and says where: from Merel and the phone,
/// as Signal does. The others in the chat keep their messages.
fn delete_chat_window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(id) = app.chat.deleting_chat.clone() else { return };
    let Some(chat) = app.chat_by_id(&id) else {
        app.chat.deleting_chat = None;
        return;
    };
    let mut yes = false;
    let closed = helder::dialog(ctx, t, &tr("chat-delete-chat-title"), 440.0, |ui| {
        ui.label(RichText::new(tr_args("chat-delete-chat-ask", &[("title", chat.title.as_str().into())])).size(14.0).color(t.text));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if helder::primary_button(ui, t, Icon::Trash, &tr("chat-delete-chat-yes")).clicked() {
                yes = true;
            }
            if helder::secondary(ui, t, |ui| ui.button(tr("cancel"))).clicked() {
                app.chat.deleting_chat = None;
            }
        });
    });
    if closed {
        app.chat.deleting_chat = None;
    }
    if yes {
        app.chat.deleting_chat = None;
        match app.engine.delete_chat(&id) {
            Ok(()) => {
                if app.chat.selected.as_deref() == Some(id.as_str()) {
                    app.chat.selected = None;
                    app.chat.replying = None;
                    app.stop_editing();
                    app.chat.files.clear();
                }
                app.notice = Some(crate::Notice::success(tr_args("chat-deleted", &[("title", chat.title.as_str().into())])));
            }
            Err(e) => app.notice = Some(crate::Notice::error(e)),
        }
    }
}

/// Above the line to write in while answering: whom and what, and a cross to stop.
fn replying_strip(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(id) = app.chat.replying.clone() else { return };
    let Some(q) = app.engine.session.chat_quote(&id) else {
        app.chat.replying = None;
        return;
    };
    let mut stop = false;
    ui.add_space(4.0);
    egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(8).inner_margin(Margin::symmetric(10, 6)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                ui.add(Icon::Reply.image(16.0, t.accent));
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr_args("chat-replying-to", &[("name", q.name.as_str().into())])).size(12.5).strong().color(t.text));
                    ui.label(RichText::new(merel_mime::preview(&q.text, 120)).size(12.5).color(t.text_dim));
                });
                helder::right(ui, |ui| {
                    let r = ui.add(egui::Button::image(Icon::X.image(14.0, t.text_dim)).frame(false));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("chat-reply-stop")));
                    stop = r.on_hover_text(tr("chat-reply-stop")).clicked();
                });
            });
        },
    );
    if stop || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.chat.replying = None;
    }
}

/// The smiley beside a message that opens the reactions; it shows while the pointer is over
/// the message or the menu is open, and keeps its place otherwise so nothing moves.
fn react_button(ui: &mut Ui, t: &Tokens, id: &str, hovered: bool, clicked: &mut Option<Clicked>) {
    let menu_id = egui::Id::new(("chat-react", id));
    let open = egui::Popup::is_id_open(ui.ctx(), menu_id);
    let size = vec2(28.0, 28.0);
    if !(hovered || open) {
        ui.allocate_exact_size(size, Sense::hover());
        return;
    }
    let button = egui::Button::new(RichText::new("☺").size(17.0).color(t.text_dim)).frame(false).min_size(size);
    let r = ui.add(button);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("chat-react")));
    egui::Popup::menu(&r).id(menu_id).show(|ui| reaction_menu(ui, clicked));
    r.on_hover_text(tr("chat-react"));
}

/// Signal's six reactions in a row, and under them the other emoji.
fn reaction_menu(ui: &mut Ui, clicked: &mut Option<Clicked>) {
    let mut pick = |ui: &mut Ui, e: &str, size: f32| {
        let b = ui.add(emoji_button_of(ui, e, size).frame(false).min_size(vec2(size + 12.0, size + 12.0)));
        b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, e));
        if b.clicked() {
            *clicked = Some(Clicked::React(e.to_string()));
            ui.close();
        }
    };
    ui.horizontal(|ui| {
        for e in QUICK_REACTIONS {
            pick(ui, e, 24.0);
        }
    });
    ui.separator();
    egui::Grid::new("chat-react-more").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
        for (i, e) in EMOJI.iter().filter(|e| !QUICK_REACTIONS.iter().any(|q| q.trim_end_matches('\u{FE0F}') == **e)).enumerate() {
            pick(ui, e, 18.0);
            if i % 8 == 7 {
                ui.end_row();
            }
        }
    });
}

/// The reactions under a message: each emoji once with how many chose it, framed in the
/// accent when one of them is the account's own. Clicking one reacts the same, or takes the
/// account's own back; hovering says who.
fn reaction_chips(ui: &mut Ui, t: &Tokens, reactions: &[merel_model::Reacted], clicked: &mut Option<Clicked>) {
    for (emoji, count) in merel_model::Reacted::tally(reactions) {
        let mine = reactions.iter().any(|r| r.mine && r.emoji == emoji);
        let who: Vec<&str> = reactions.iter().filter(|r| r.emoji == emoji).map(|r| r.name.as_str()).collect();
        let button = match crate::emoji::image_of(ui.ctx(), &emoji, 16.0) {
            Some(img) if count > 1 => egui::Button::image_and_text(img, RichText::new(count.to_string()).size(13.0).color(t.text)),
            Some(img) => egui::Button::image(img),
            None if count > 1 => egui::Button::new(RichText::new(format!("{emoji} {count}")).size(14.0).color(t.text)),
            None => egui::Button::new(RichText::new(emoji.as_str()).size(14.0).color(t.text)),
        };
        let button = button
            .fill(if mine { t.checked } else { t.card })
            .stroke(egui::Stroke::new(1.0, if mine { t.accent } else { t.card_border }))
            .corner_radius(10)
            .min_size(vec2(32.0, 24.0));
        let r = ui.add(button);
        let names = who.join(", ");
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{emoji} {names}")));
        if r.on_hover_text(names).clicked() {
            *clicked = Some(Clicked::React(emoji.clone()));
        }
    }
}

/// A message of one's own that waits for a key: dimmed, with what it waits for under it.
fn waiting_bubble(ui: &mut Ui, t: &Tokens, chat: &Chat, w: &Waiting) {
    let max = (ui.available_width() * 0.7).min(480.0);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 0.0), Layout::top_down(Align::Max), |ui| {
        egui::Frame::new()
            .fill(t.checked.gamma_multiply(0.5))
            .stroke(egui::Stroke::new(1.0, t.card_border))
            .corner_radius(12)
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_max_width(max);
                ui.add(egui::Label::new(RichText::new(w.draft.body.trim()).size(14.0).color(t.text)).wrap());
            });
        ui.horizontal(|ui| {
            ui.add(Icon::Clock.image(13.0, t.text_dim));
            ui.label(RichText::new(tr_args("chat-waiting", &[("name", chat.title.as_str().into())])).size(12.0).color(t.text_dim));
        });
    });
    ui.add_space(6.0);
}

/// The line to write in: a rounded box with the text and the send button inside it.
fn composer(app: &mut MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) {
    ui.add_space(6.0);
    let mut send = false;
    let mut pick = false;
    let mut record = false;
    let signal = app.engine.is_signal_account(&chat.account_id);
    if app.chat.recording.is_some() {
        recording_bar(app, ui, t);
        return;
    }
    egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(10).inner_margin(Margin::symmetric(12, 6)).show(
        ui,
        |ui| {
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 36.0), Layout::left_to_right(Align::Center), |ui| {
                let width = (ui.available_width() - if signal { 116.0 } else { 82.0 }).max(120.0);
                let hint = tr_args("chat-input-hint", &[("name", chat.title.as_str().into())]);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.chat.input)
                        .id(egui::Id::new(INPUT))
                        .frame(egui::Frame::NONE)
                        .hint_text(RichText::new(hint).color(t.text_dim))
                        .desired_width(width)
                        .margin(Margin::symmetric(0, 8)),
                );
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, tr("chat-input")));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                emoji_button(ui, t, &mut app.chat.input);
                // Files go over Signal; chat over mail stays text, as Delta Chat keeps it.
                if signal && app.chat.editing.is_none() {
                    let r = ui.add(egui::Button::image(Icon::Paperclip.image(18.0, t.text_dim)).frame(false).min_size(vec2(30.0, 30.0)));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("chat-attach")));
                    pick = r.on_hover_text(tr("chat-attach")).clicked();
                }
                let ready = !app.chat.input.trim().is_empty() || !app.chat.files.is_empty();
                // With nothing typed, Signal's microphone takes the send button's place.
                if signal && !ready && app.chat.editing.is_none() {
                    record = mic_button(ui, t).clicked();
                } else {
                    let clicked = ui.add_enabled_ui(ready, |ui| send_button(ui, t)).inner.clicked();
                    send = (enter || clicked) && ready;
                }
            });
        },
    );
    if pick && let Some(paths) = rfd::FileDialog::new().set_title(tr("chat-attach")).pick_files() {
        app.chat.error = None;
        add_chat_files(app, paths);
    }
    if send {
        app.send_in_chat();
        ui.memory_mut(|m| m.request_focus(egui::Id::new(INPUT)));
    }
    if record {
        app.chat.voice.stop();
        match merel_voice::record() {
            Ok(r) => {
                app.chat.recording = Some(r);
                app.chat.error = None;
            }
            Err(e) => app.chat.error = Some(tr_args("chat-voice-no-mic", &[("error", e.into())])),
        }
    }
}

/// While a voice message is recorded: a red dot, the time, how loud it is, and buttons to
/// throw it away or send it.
fn recording_bar(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(recording) = &app.chat.recording else { return };
    let elapsed = recording.elapsed();
    let level = recording.level();
    let mut cancel = false;
    let mut send = false;
    egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(10).inner_margin(Margin::symmetric(12, 6)).show(
        ui,
        |ui| {
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 36.0), Layout::left_to_right(Align::Center), |ui| {
                let r = ui.add(egui::Button::image(Icon::X.image(18.0, t.text_dim)).frame(false).min_size(vec2(30.0, 30.0)));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("chat-voice-cancel")));
                cancel = r.on_hover_text(tr("chat-voice-cancel")).clicked();
                ui.add_space(6.0);
                // The dot blinks, as on a recorder.
                let (dot, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                if elapsed.as_millis() % 1000 < 600 {
                    ui.painter().circle_filled(dot.center(), 6.0, t.flag);
                }
                ui.add_space(6.0);
                let s = elapsed.as_secs();
                let time = format!("{}:{:02}", s / 60, s % 60);
                ui.label(RichText::new(format!("{}  {time}", tr("chat-voice-recording"))).size(14.0).color(t.text));
                ui.add_space(10.0);
                // How loud: a bar that grows, so one sees the microphone hears.
                let width = (ui.available_width() - 50.0).clamp(40.0, 240.0);
                let (bar, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
                ui.painter().rect_filled(bar, 3.0, t.card_border);
                let loud = egui::Rect::from_min_size(bar.min, vec2(bar.width() * level.sqrt().min(1.0), bar.height()));
                ui.painter().rect_filled(loud, 3.0, t.accent);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| send = send_button(ui, t).clicked());
            });
        },
    );
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    if cancel {
        app.chat.recording = None;
    } else if send || elapsed >= merel_voice::MAX_LENGTH {
        app.send_voice();
    }
}

/// The microphone button: records a voice message.
fn mic_button(ui: &mut Ui, t: &Tokens) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("chat-voice-record")));
    let fill = if r.hovered() { t.accent.gamma_multiply(0.9) } else { t.accent };
    ui.painter().rect_filled(rect, 8.0, fill);
    Icon::Mic.image(18.0, t.on_accent).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(18.0, 18.0)));
    r.on_hover_text(tr("chat-voice-record"))
}

/// The emoji most used in chats, for the button next to the line to write in.
const EMOJI: [&str; 32] = [
    "😀", "😂", "🙂", "😉", "😍", "🥰", "😘", "😎", "🤔", "😅", "😢", "😮", "😡", "🥳", "🙏", "👍", "👎", "👏", "🙌", "💪", "👋", "❤", "🔥", "🎉",
    "✅", "❌", "⭐", "☕", "🍻", "🥂", "🎂", "🌹",
];

/// A button showing emoji `e` in colour, `size` points.
fn emoji_button_of(ui: &Ui, e: &str, size: f32) -> egui::Button<'static> {
    match crate::emoji::image_of(ui.ctx(), e, size) {
        Some(img) => egui::Button::image(img),
        None => egui::Button::new(RichText::new(e.to_string()).size(size)),
    }
}

/// Every emoji the pickers offer.
#[cfg(test)]
pub(crate) fn all_emoji() -> impl Iterator<Item = &'static str> {
    EMOJI.into_iter().chain(QUICK_REACTIONS)
}

/// A smiley that opens the emoji to add to `input`; the computer's own picker works as well.
fn emoji_button(ui: &mut Ui, t: &Tokens, input: &mut String) {
    let r = ui.menu_button(RichText::new("🙂").size(19.0).color(t.text_dim), |ui| {
        egui::Grid::new("chat-emoji").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
            for (i, e) in EMOJI.iter().enumerate() {
                let b = ui.add(emoji_button_of(ui, e, 20.0).frame(false).min_size(vec2(32.0, 32.0)));
                b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, *e));
                if b.clicked() {
                    input.push_str(e);
                    ui.close();
                }
                if i % 8 == 7 {
                    ui.end_row();
                }
            }
        });
    });
    r.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("chat-emoji")));
    r.response.on_hover_text(tr("chat-emoji"));
}

/// The square blue send button with the paper plane.
fn send_button(ui: &mut Ui, t: &Tokens) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tr("chat-send")));
    let fill = if !ui.is_enabled() {
        t.card_border
    } else if r.hovered() {
        t.accent.gamma_multiply(0.9)
    } else {
        t.accent
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    Icon::Send.image(18.0, t.on_accent).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(18.0, 18.0)));
    r.on_hover_text(tr("chat-send"))
}

/// A card on the grey ground, with a title.
fn section<R>(ui: &mut Ui, t: &Tokens, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    helder::card_frame(t)
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(17.0).color(t.text));
            ui.add_space(6.0);
            add(ui)
        })
        .inner
}

/// What the right side shows without an open chat: what chat is, and Nieuwe chat; without a
/// Signal account, the way to link one.
fn start_page(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let linked = !app.chat_accounts().is_empty();
    section(ui, t, &tr("chat-title"), |ui| {
        helder::paragraph(ui, t, &tr("chat-intro"));
        ui.add_space(12.0);
        if !linked {
            helder::note(ui, t, &tr("chat-link-signal-first"));
            ui.add_space(10.0);
            if helder::primary_button(ui, t, Icon::Link, &tr("signal-link")).clicked() {
                app.open_signal_link();
            }
            return;
        }
        if let Some(e) = &app.chat.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        if helder::primary_button(ui, t, Icon::MessagesSquare, &tr("chat-new")).clicked() {
            app.new_chat();
        }
    });
}

/// A QR code of `text`, `size` points wide: dark on white whatever the theme, with the quiet
/// zone around it that scanners need.
pub(crate) fn qr_code(ui: &mut Ui, text: &str, size: f32, label: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, label));
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else { return };
    let width = code.width();
    let colors = code.to_colors();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, egui::Color32::WHITE);
    painter.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, t_border(ui)), egui::StrokeKind::Inside);
    let cell = size / (width + 4) as f32;
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (x, y) = ((i % width + 2) as f32, (i / width + 2) as f32);
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min + vec2(x * cell, y * cell), vec2(cell + 0.5, cell + 0.5)),
                0.0,
                egui::Color32::BLACK,
            );
        }
    }
}

/// A light line around the white of the QR code, so it keeps its edge on a white card.
fn t_border(ui: &Ui) -> egui::Color32 {
    ui.visuals().widgets.noninteractive.bg_stroke.color
}

/// The Signal contacts that match what is typed, by name or number, leaving out who is
/// picked already; the first few when nothing is typed yet.
fn suggestions(app: &MailApp, form: &NewChat) -> Vec<(Address, String)> {
    if !form.people.is_empty() {
        return Vec::new();
    }
    let needle = form.typed.trim().to_lowercase();
    let digits: String = needle.chars().filter(char::is_ascii_digit).collect();
    app.engine
        .signal_contacts(&form.account_id)
        .into_iter()
        .filter(|(a, number)| {
            needle.is_empty()
                || a.display().to_lowercase().contains(&needle)
                || (!digits.is_empty() && number.chars().filter(char::is_ascii_digit).collect::<String>().contains(&digits))
        })
        .take(MAX_SUGGESTIONS)
        .collect()
}

/// A person picked for the chat: avatar, name and a cross to take them out again. True when
/// the cross is clicked.
fn person_chip(ui: &mut Ui, t: &Tokens, person: &Address) -> bool {
    let mut remove = false;
    egui::Frame::new().fill(t.checked).corner_radius(14).inner_margin(Margin { left: 3, right: 4, top: 2, bottom: 2 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        helder::avatar(ui.painter(), rect.center(), 11.0, person.display(), person.email());
        ui.label(RichText::new(person.display()).size(13.5).color(t.accent_text));
        let label = tr_args("chat-remove-person", &[("name", person.display().into())]);
        let (rect, r) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if r.hovered() {
            ui.painter().circle_filled(rect.center(), 9.0, t.hover);
        }
        Icon::X.image(12.0, t.accent_text).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(12.0, 12.0)));
        remove = r.on_hover_text(label).clicked();
    });
    remove
}

/// One suggestion under the people field: avatar, name and number. True when clicked.
fn suggestion_row(ui: &mut Ui, t: &Tokens, person: &Address, number: &str) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, person.display()));
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect, 8, t.hover);
    }
    helder::avatar(p, egui::pos2(rect.left() + 24.0, rect.center().y), 16.0, person.display(), person.email());
    let x = rect.left() + 50.0;
    p.text(egui::pos2(x, rect.top() + 7.0), egui::Align2::LEFT_TOP, person.display(), FontId::proportional(14.0), t.text);
    let line = if number.is_empty() { tr("signal-contact") } else { format!("{number} · {}", tr("signal-contact")) };
    p.text(egui::pos2(x, rect.top() + 25.0), egui::Align2::LEFT_TOP, line, FontId::proportional(12.0), t.text_dim);
    r.clicked()
}

/// The new-chat window: the command bar (Starten, the Signal account when there are more,
/// Annuleren), the person as a chip with suggestions from the Signal contacts, and the first
/// message.
fn new_chat_form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let accounts: Vec<(String, String)> = app
        .engine
        .session
        .accounts
        .iter()
        .filter(|a| merel_engine::signal::is_signal(a))
        .map(|a| (a.id.clone(), if a.name.trim().is_empty() { a.address.display().to_string() } else { a.name.clone() }))
        .collect();
    let suggested = app.chat.new.as_ref().map(|f| suggestions(app, f)).unwrap_or_default();
    let contacts = app.chat.new.as_ref().map(|f| app.engine.signal_contacts(&f.account_id)).unwrap_or_default();
    let mut start = false;
    let mut cancel = false;
    let Some(form) = app.chat.new.as_mut() else { return };
    helder::quiet(ui, t, |ui| {
        ui.horizontal(|ui| {
            let can_start = !form.account_id.is_empty();
            start = ui.add_enabled_ui(can_start, |ui| helder::primary_button(ui, t, Icon::Send, &tr("chat-start"))).inner.clicked();
            // Which Signal account, only when there is more than one.
            if accounts.len() > 1 {
                helder::bar_separator(ui, t);
                let current =
                    accounts.iter().find(|(id, _)| *id == form.account_id).map(|(_, n)| n.clone()).unwrap_or_else(|| tr("chat-pick-account"));
                helder::menu_button(ui, t, Some(Icon::MessagesSquare), RichText::new(current).size(14.0).color(t.text), |ui| {
                    for (id, name) in &accounts {
                        if ui.selectable_label(form.account_id == *id, name).clicked() {
                            form.account_id = id.clone();
                            form.people.clear();
                            ui.close();
                        }
                    }
                });
            }
            helder::trailing(ui, |ui| {
                cancel = helder::tool_button(ui, t, Icon::X, &tr("chat-cancel")).clicked();
            });
        });
    });
    ui.add_space(8.0);
    helder::card_frame(t).inner_margin(Margin::same(20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let width = (ui.available_width() - 48.0).max(200.0);
        // The people: chips, then the field to type more in.
        helder::icon_row(ui, t, Icon::Users, |ui| {
            ui.vertical(|ui| {
                ui.set_width(width);
                let mut remove = None;
                let mut typed_r = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for (i, p) in form.people.iter().enumerate() {
                        if person_chip(ui, t, p) {
                            remove = Some(i);
                        }
                    }
                    let hint = if form.people.is_empty() { tr("chat-people-hint") } else { String::new() };
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut form.typed)
                            .id(egui::Id::new(PEOPLE))
                            .frame(egui::Frame::NONE)
                            .hint_text(RichText::new(hint).color(t.text_dim))
                            .desired_width((ui.available_size_before_wrap().x - 8.0).max(160.0))
                            .margin(Margin::symmetric(2, 6)),
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, tr("chat-people")));
                    typed_r = Some(r);
                });
                let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
                let focused = typed_r.as_ref().is_some_and(egui::Response::has_focus);
                ui.painter().hline(
                    line.x_range(),
                    line.center().y,
                    egui::Stroke::new(if focused { 2.0 } else { 1.0 }, if focused { t.accent } else { t.card_border }),
                );
                if let Some(i) = remove {
                    form.people.remove(i);
                }
                if let Some(r) = typed_r {
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let comma = form.typed.ends_with([',', ';']);
                    let backspace = r.has_focus() && form.typed.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Backspace));
                    if backspace {
                        form.people.pop();
                    } else if enter || comma {
                        match suggested.first().filter(|_| !comma && !form.typed.trim().is_empty()) {
                            Some((first, _)) => {
                                add_person(form, first.clone());
                                form.typed.clear();
                            }
                            None => {
                                commit_typed(form, &contacts);
                            }
                        }
                        ui.memory_mut(|m| m.request_focus(egui::Id::new(PEOPLE)));
                    }
                }
                if !suggested.is_empty() {
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(t.card)
                        .stroke(egui::Stroke::new(1.0, t.card_border))
                        .corner_radius(10)
                        .inner_margin(Margin::same(6))
                        .show(ui, |ui| {
                            ui.set_width(width.min(420.0));
                            ui.label(RichText::new(tr("chat-suggest")).size(12.0).color(t.text_dim));
                            for (person, number) in &suggested {
                                if suggestion_row(ui, t, person, number) {
                                    add_person(form, person.clone());
                                    form.typed.clear();
                                    ui.memory_mut(|m| m.request_focus(egui::Id::new(PEOPLE)));
                                }
                            }
                        });
                }
            });
        });
        ui.add_space(4.0);
        helder::icon_row(ui, t, Icon::Notes, |ui| {
            egui::Frame::new().fill(t.window).corner_radius(helder::CONTROL_RADIUS).inner_margin(Margin::same(10)).show(ui, |ui| {
                let r = ui.add(
                    egui::TextEdit::multiline(&mut form.text)
                        .frame(egui::Frame::NONE)
                        .hint_text(RichText::new(tr("chat-first-hint")).color(t.text_dim))
                        .desired_width(width - 20.0)
                        .desired_rows(6),
                );
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, tr("chat-first-hint")));
            });
        });
        if let Some(e) = &form.error {
            ui.add_space(6.0);
            helder::icon_row(ui, t, Icon::CircleAlert, |ui| {
                ui.label(RichText::new(e).color(t.flag));
            });
        }
        ui.add_space(10.0);
        helder::icon_row(ui, t, Icon::ShieldCheck, |ui| {
            ui.add(egui::Label::new(RichText::new(tr("chat-new-note")).size(12.5).color(t.text_dim)).wrap());
        });
    });
    if cancel {
        app.chat.new = None;
    } else if start {
        app.start_chat();
    }
}

/// The ribbon of the chat module.
pub fn ribbon(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    if crate::ribbon_action(ui, t, "💬", &tr("chat-new"), true) {
        app.new_chat();
    }
    crate::ribbon_separator(ui);
    let open = app.chat.selected.is_some();
    if crate::ribbon_action(ui, t, "◎", &tr("chat-all"), open) {
        app.chat.selected = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_show_the_hour_today_and_the_day_before() {
        let now = Local::now().timestamp();
        assert_eq!(when(now).len(), 5);
        assert!(when(now).contains(':'));
        assert!(when(now - 3 * 86_400).contains('-'));
    }

    #[test]
    fn an_invitation_is_found_in_a_message() {
        let text = "Ik wil graag chatten. Open deze link:\n\nhttps://i.delta.chat/#ABC&i=x&s=y&a=a%40b.nl\n";
        assert_eq!(invite_in(text), Some("https://i.delta.chat/#ABC&i=x&s=y&a=a%40b.nl"));
        assert_eq!(invite_in("Gewoon een bericht"), None);
    }

    #[test]
    fn a_typed_name_or_number_of_a_signal_contact_becomes_a_chip_and_nothing_else_does() {
        let sanne = Address::new(Some("Sanne"), "abc@signal.invalid").unwrap();
        let contacts = vec![(sanne.clone(), "+31 6 12345678".to_string())];
        let mut form = NewChat { typed: "sanne,".into(), ..NewChat::default() };
        assert!(commit_typed(&mut form, &contacts));
        assert_eq!(form.people, vec![sanne.clone()]);
        form.typed = "+31612345678".into();
        assert!(commit_typed(&mut form, &contacts));
        assert_eq!(form.people.len(), 1, "once");
        form.typed = "sanne@example.org".into();
        assert!(!commit_typed(&mut form, &contacts), "an e-mail address is no Signal contact");
        assert!(form.error.is_some());
    }
}
