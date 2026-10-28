//! The chat module: the chats on the left, the open one on the right with its messages and a
//! line to write, and a window for a new chat that picks people from the address book. Chat
//! goes over e-mail, Delta Chat's way, and only end-to-end encrypted; see `merel_engine::chat`.
//! Someone without a key yet gets an invitation by mail from Merel itself, and the message
//! waits until their key comes (`merel_engine::securejoin`).

use chrono::{Local, NaiveDate};
use egui::{Align, FontId, Layout, Margin, RichText, Sense, Ui, vec2};

use merel_engine::chat::{self as engine_chat, Chat};
use merel_engine::securejoin::{INVITE_LINK, Waiting};
use merel_engine::{SAMPLE_ACCOUNT, SendState};
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
        let account_id = self.chat_accounts().into_iter().find(|id| self.engine.chat_ready(id)).or_else(|| self.chat_accounts().into_iter().next());
        self.chat.new = Some(NewChat { account_id: account_id.unwrap_or_default(), ..NewChat::default() });
        self.chat.error = None;
    }

    /// Opens the chat with `person`, as the contacts module offers it: the one already
    /// going, else the window for a new chat with them.
    pub fn chat_with(&mut self, person: Address) {
        self.module = crate::Module::Chat;
        let existing = self
            .engine
            .session
            .chats()
            .into_iter()
            .find(|c| !c.is_group() && c.people.iter().any(|p| p.email().eq_ignore_ascii_case(person.email())));
        if let Some(chat) = existing {
            self.open_chat(&chat.id);
            return;
        }
        self.new_chat();
        if let Some(form) = self.chat.new.as_mut() {
            form.people.push(person);
        }
    }

    /// Whether any account can carry chat.
    pub(crate) fn can_chat(&self) -> bool {
        !self.chat_accounts().is_empty()
    }

    /// The accounts that can carry chat, by id.
    fn chat_accounts(&self) -> Vec<String> {
        self.engine.session.accounts.iter().filter(|a| engine_chat::can_chat(a)).map(|a| a.id.clone()).collect()
    }

    fn open_chat(&mut self, id: &str) {
        self.chat.selected = Some(id.to_string());
        self.chat.new = None;
        self.chat.error = None;
        self.engine.open_chat(id);
    }

    /// The chat `id`: one with messages, or one whose first message waits for a key.
    fn chat_by_id(&self, id: &str) -> Option<Chat> {
        self.engine.session.chat(id).or_else(|| waiting_chats(self).into_iter().find(|c| c.id == id))
    }

    /// Sends what is typed in the open chat.
    fn send_in_chat(&mut self) {
        let Some(chat) = self.chat.selected.as_deref().and_then(|id| self.chat_by_id(id)) else { return };
        let text = self.chat.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let draft = self.engine.session.chat_reply(&chat, &text);
        match self.engine.send_or_invite(&chat.account_id, draft) {
            Ok(id) => {
                self.chat.sending = id;
                self.chat.input.clear();
                self.chat.error = None;
            }
            Err(e) => self.chat.error = Some(e),
        }
    }

    /// Shows the invitation link of `account_id`, turning chat on for it if need be.
    fn show_invite(&mut self, account_id: &str, renew: bool) {
        let link = if renew { self.engine.renew_invite_link(account_id) } else { self.engine.invite_link(account_id) };
        match link {
            Ok(link) => {
                self.chat.invite = Some((account_id.to_string(), link));
                self.chat.error = None;
            }
            Err(e) => self.chat.error = Some(e),
        }
    }

    /// Opens a new message with the invitation link, for someone to get it by mail.
    fn mail_invite(&mut self) {
        let Some((account_id, link)) = self.chat.invite.clone() else { return };
        let signature = self.engine.session.accounts.iter().find(|a| a.id == account_id).map(|a| a.signature.clone()).unwrap_or_default();
        let mut c = crate::compose::Compose::new(crate::compose::Kind::New, None).with_signature(&signature);
        c.subject = tr("chat-invite-mail-subject");
        c.body = format!("{}\n\n{link}{}", tr("chat-invite-mail-text"), c.body);
        self.compose = Some(c.opened_from(&account_id));
    }

    /// Accepts the invitation link pasted on the start page.
    fn accept_invite(&mut self) {
        let account_id = if self.chat.accept_account.is_empty() {
            self.chat_accounts().into_iter().next().unwrap_or_default()
        } else {
            self.chat.accept_account.clone()
        };
        let text = self.chat.accept.trim().to_string();
        self.chat.accept_note = Some(self.take_invite(&account_id, &text));
        if matches!(self.chat.accept_note, Some(Ok(_))) {
            self.chat.accept.clear();
        }
    }

    /// Accepts the invitation `link` that came by mail to `account_id`, and shows the chats'
    /// start page with what happens next. In a browser the link leads to Delta Chat's own page,
    /// which only Delta Chat can follow; read in Merel it is taken here.
    pub fn open_invite(&mut self, account_id: &str, link: &str) {
        let chat_accounts = self.chat_accounts();
        let account_id = if chat_accounts.iter().any(|id| id == account_id) {
            account_id.to_string()
        } else {
            chat_accounts.into_iter().next().unwrap_or_default()
        };
        let account_id = account_id.as_str();
        self.module = crate::Module::Chat;
        self.chat.selected = None;
        self.chat.accept_account = account_id.to_string();
        self.chat.accept_note = Some(self.take_invite(account_id, link));
    }

    /// Accepts the invitation `link` on `account_id`; says what happens next.
    fn take_invite(&mut self, account_id: &str, link: &str) -> Result<String, String> {
        self.engine.accept_invite(account_id, link)?;
        let name = merel_engine::securejoin::parse_invite(link).map(|i| if i.name.is_empty() { i.addr } else { i.name }).unwrap_or_default();
        Ok(tr_args("chat-accept-sent", &[("name", name.into())]))
    }

    /// Starts the chat the window describes: sends the first message, or holds it while an
    /// invitation goes to whoever has no key yet.
    fn start_chat(&mut self) {
        let Some(form) = self.chat.new.as_mut() else { return };
        if !commit_typed(form) {
            return;
        }
        let name = if form.group { form.name.trim().to_string() } else { String::new() };
        let draft = match engine_chat::new_chat(&form.people, &name, &form.text) {
            Ok(d) => d,
            Err(e) => {
                form.error = Some(e);
                return;
            }
        };
        let account_id = form.account_id.clone();
        // The first chat from an account makes its key: chat needs one, and nobody should have
        // to find a switch for that first.
        if !self.engine.chat_ready(&account_id)
            && let Err(e) = self.engine.enable_chat(&account_id)
        {
            if let Some(form) = self.chat.new.as_mut() {
                form.error = Some(e);
            }
            return;
        }
        let id = match draft.chat.as_ref().and_then(|c| c.group_id.as_deref()) {
            Some(group) => engine_chat::group_chat_id(&account_id, group),
            None => engine_chat::person_chat_id(&account_id, draft.to.first().map(Address::email).unwrap_or("")),
        };
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

/// Turns what is typed in the people field into a chip, when it is an address; false (with
/// the reason in the form) when it is something else.
fn commit_typed(form: &mut NewChat) -> bool {
    let typed = form.typed.trim().trim_end_matches([',', ';']).trim().to_string();
    if typed.is_empty() {
        return true;
    }
    match Address::new(None, &typed) {
        Ok(a) => {
            add_person(form, a);
            form.typed.clear();
            true
        }
        Err(_) => {
            form.error = Some(tr("chat-not-an-address"));
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

/// The chats whose first message waits for a key and that have no messages yet, as chats.
fn waiting_chats(app: &MailApp) -> Vec<Chat> {
    let mut out: Vec<Chat> = Vec::new();
    for w in app.engine.waiting() {
        let id = w.chat_id();
        if out.iter().any(|c| c.id == id) || app.engine.session.chat(&id).is_some() {
            continue;
        }
        let group = w.draft.chat.clone().filter(|c| c.group_id.is_some());
        let title = group
            .as_ref()
            .and_then(|g| g.group_name.clone())
            .unwrap_or_else(|| w.draft.to.iter().map(|a| a.display().to_string()).collect::<Vec<_>>().join(", "));
        out.push(Chat {
            id,
            account_id: w.account_id.clone(),
            title,
            people: w.draft.to.clone(),
            group,
            messages: Vec::new(),
            last: w.since,
            unread: 0,
        });
    }
    out
}

/// The messages of `chat` that wait for a key.
fn waiting_in<'a>(app: &'a MailApp, chat: &Chat) -> Vec<&'a Waiting> {
    app.engine.waiting().iter().filter(|w| w.chat_id() == chat.id).collect()
}

/// The chats the list shows, by the filter and the search box.
fn listed(app: &MailApp) -> Vec<Chat> {
    let needle = app.chat.query.trim().to_lowercase();
    let mut chats = waiting_chats(app);
    chats.extend(app.engine.session.chats());
    chats
        .into_iter()
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
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for chat in &list {
                    if chat_row(app, ui, t, chat).clicked() {
                        pick = Some(chat.id.clone());
                    }
                }
            });
            if let Some(id) = pick {
                app.open_chat(&id);
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
        // Calls go over mail; a Signal chat has none in Merel.
        if !signal_chat {
            helder::right(ui, |ui| call_button(app, ui, t, chat));
        }
    });
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, t.card_border));
    ui.add_space(4.0);
    // The line to write sits at the bottom; the messages fill what is above it.
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        composer(app, ui, t, chat);
        if let Some(e) = &app.chat.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        if let Some(SendState::Failed(e)) = app.chat.sending.and_then(|id| app.engine.session.outbox.get(&id)) {
            ui.label(RichText::new(tr_args("chat-not-sent", &[("reason", e.as_str().into())])).color(t.flag));
        }
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            let mut accept = None;
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
                    if let Some(link) = bubble(app, ui, t, chat, id, first_of_run) {
                        accept = Some(link);
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
            if let Some(link) = accept {
                let account = chat.account_id.clone();
                match app.take_invite(&account, &link) {
                    Ok(_) => app.chat.error = None,
                    Err(e) => app.chat.error = Some(e),
                }
            }
        });
    });
}

/// Bellen, or Deelnemen while the others are in a meeting of this chat, or Terug naar het
/// gesprek while this computer is in one.
fn call_button(app: &mut MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) {
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

/// One message: on the right in the accent's tint when it is one's own, else on the left
/// with the sender's avatar and name over the first of a run. An invitation shows as one,
/// with a button to accept it; gives its link when that is clicked.
fn bubble(app: &MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat, id: &str, first_of_run: bool) -> Option<String> {
    let session = &app.engine.session;
    let m = session.message(id)?;
    let mine = session.chat_is_mine(id);
    let text = session.chat_text(id).unwrap_or_else(|| tr("chat-loading"));
    let invite = invite_in(&text).map(str::to_string);
    let ring = m.chat.as_ref().and_then(|c| c.call.as_ref()).is_some_and(|c| c.step == merel_model::CallStep::Ring);
    let max = (ui.available_width() * 0.7).min(480.0);
    let mut accept = None;
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
        ui.horizontal_top(|ui| {
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
                    match &invite {
                        Some(link) => {
                            ui.horizontal(|ui| {
                                ui.add(Icon::Link.image(16.0, t.text));
                                let line =
                                    if mine { tr("chat-invite-sent") } else { tr_args("chat-invite-received", &[("name", m.from.display().into())]) };
                                ui.add(egui::Label::new(RichText::new(line).size(14.0).color(t.text)).wrap());
                            });
                            if !mine && !app.engine.is_verified(m.from.email()) {
                                ui.add_space(6.0);
                                if helder::primary_button(ui, t, Icon::MessagesSquare, &tr("chat-accept-button")).clicked() {
                                    accept = Some(link.clone());
                                }
                            }
                        }
                        None if ring => {
                            ui.horizontal(|ui| {
                                ui.add(Icon::Phone.image(16.0, t.text));
                                let line =
                                    if mine { tr("call-ring-mine") } else { tr_args("call-ring-theirs", &[("name", m.from.display().into())]) };
                                ui.add(egui::Label::new(RichText::new(line).size(14.0).color(t.text)).wrap());
                            });
                        }
                        None => {
                            ui.add(egui::Label::new(RichText::new(text.as_str()).size(14.0).color(t.text)).wrap());
                        }
                    }
                },
            );
        });
    });
    let _ = chat;
    ui.add_space(4.0);
    accept
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
    egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(10).inner_margin(Margin::symmetric(12, 6)).show(
        ui,
        |ui| {
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 36.0), Layout::left_to_right(Align::Center), |ui| {
                let width = (ui.available_width() - 44.0).max(120.0);
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
                let ready = !app.chat.input.trim().is_empty();
                let clicked = ui.add_enabled_ui(ready, |ui| send_button(ui, t)).inner.clicked();
                send = (enter || clicked) && ready;
            });
        },
    );
    if send {
        app.send_in_chat();
        ui.memory_mut(|m| m.request_focus(egui::Id::new(INPUT)));
    }
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

/// What the right side shows without an open chat: chat to turn on, the invitation link, and
/// a link someone sent to accept.
fn start_page(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let accounts: Vec<(String, String)> =
        app.engine.session.accounts.iter().filter(|a| engine_chat::can_chat(a)).map(|a| (a.id.clone(), a.address.email().to_string())).collect();
    section(ui, t, &tr("chat-title"), |ui| {
        helder::paragraph(ui, t, &tr("chat-intro"));
        ui.add_space(12.0);
        if accounts.is_empty() {
            let key = if app.engine.session.accounts.iter().all(|a| a.id == SAMPLE_ACCOUNT) { "chat-add-account" } else { "chat-imap-only" };
            helder::note(ui, t, &tr(key));
            return;
        }
        for (id, email) in &accounts {
            account_row(app, ui, t, id, email);
        }
        if let Some(e) = &app.chat.error {
            ui.label(RichText::new(e).color(t.flag));
        }
        ui.add_space(10.0);
        if helder::primary_button(ui, t, Icon::MessagesSquare, &tr("chat-new")).clicked() {
            app.new_chat();
        }
    });
    if accounts.is_empty() {
        return;
    }
    if app.chat.invite.is_some() {
        ui.add_space(12.0);
        invite_card(app, ui, t);
    }
    ui.add_space(12.0);
    accept_card(app, ui, t, &accounts);
}

/// An account's line on the start page: chat on with its invitation link, or a button to
/// turn it on.
fn account_row(app: &mut MailApp, ui: &mut Ui, t: &Tokens, id: &str, email: &str) {
    egui::Frame::new().fill(t.window).corner_radius(helder::CONTROL_RADIUS).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let ready = app.engine.chat_ready(id);
        let height = if ready { helder::BUTTON_HEIGHT } else { 0.0 };
        ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::left_to_right(if ready { Align::Center } else { Align::Min }), |ui| {
            ui.add(Icon::Lock.image(18.0, t.text));
            ui.add_space(6.0);
            if app.engine.chat_ready(id) {
                ui.label(RichText::new(tr_args("chat-on", &[("email", email.into())])).color(t.text));
                helder::trailing(ui, |ui| {
                    if helder::secondary(ui, t, |ui| helder::tool_button(ui, t, Icon::Link, &tr("chat-invite"))).clicked() {
                        app.show_invite(id, false);
                    }
                });
            } else {
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr_args("chat-off", &[("email", email.into())])).color(t.text));
                    ui.add(egui::Label::new(RichText::new(tr("chat-key-note")).size(12.5).color(t.text_dim)).wrap());
                    ui.add_space(4.0);
                    if helder::secondary(ui, t, |ui| ui.button(tr_args("chat-enable", &[("email", email.into())]))).clicked() {
                        match app.engine.enable_chat(id) {
                            Ok(_) => app.chat.error = None,
                            Err(e) => app.chat.error = Some(e),
                        }
                    }
                });
            }
        });
    });
    ui.add_space(6.0);
}

/// The invitation link of an account: its QR code to scan, the link to copy or mail, and a
/// way to make a new one.
fn invite_card(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let Some((account_id, link)) = app.chat.invite.clone() else { return };
    let email = app.engine.session.accounts.iter().find(|a| a.id == account_id).map(|a| a.address.email().to_string()).unwrap_or_default();
    helder::card_frame(t).inner_margin(Margin::same(20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr_args("chat-invite-for", &[("email", email.as_str().into())])).size(17.0).color(t.text));
            helder::trailing(ui, |ui| {
                if helder::quiet(ui, t, |ui| helder::icon_button(ui, t, Icon::X, &tr("chat-invite-close"))).clicked() {
                    app.chat.invite = None;
                }
            });
        });
        ui.add_space(4.0);
        helder::paragraph(ui, t, &tr("chat-invite-intro"));
        ui.add_space(12.0);
        ui.horizontal_top(|ui| {
            qr_code(ui, &link, 188.0, &tr("chat-invite-qr"));
            ui.add_space(20.0);
            ui.vertical(|ui| {
                egui::Frame::new().fill(t.window).corner_radius(helder::CONTROL_RADIUS).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(&link).font(FontId::monospace(11.5)).color(t.text_dim)).wrap());
                });
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    let now = ui.input(|i| i.time);
                    let copied = app.chat.copied_at.is_some_and(|at| now - at < 2.0);
                    let copy = if copied { tr("chat-invite-copied") } else { tr("chat-invite-copy") };
                    if helder::primary_button(ui, t, Icon::Copy, &copy).clicked() {
                        ui.ctx().copy_text(link.clone());
                        app.chat.copied_at = Some(now);
                    }
                    if helder::secondary(ui, t, |ui| helder::tool_button(ui, t, Icon::Mail, &tr("chat-invite-mail"))).clicked() {
                        app.mail_invite();
                    }
                    if helder::secondary(ui, t, |ui| helder::tool_button(ui, t, Icon::RefreshCw, &tr("chat-invite-renew")))
                        .on_hover_text(tr("chat-invite-renew-hint"))
                        .clicked()
                    {
                        app.show_invite(&account_id, true);
                    }
                });
            });
        });
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

/// Where a link someone sent is pasted to accept it, and the invitations waiting for an answer.
fn accept_card(app: &mut MailApp, ui: &mut Ui, t: &Tokens, accounts: &[(String, String)]) {
    section(ui, t, &tr("chat-accept"), |ui| {
        helder::paragraph(ui, t, &tr("chat-accept-intro"));
        ui.add_space(10.0);
        if accounts.len() > 1 {
            helder::icon_row(ui, t, Icon::Mail, |ui| {
                let current = accounts
                    .iter()
                    .find(|(id, _)| *id == app.chat.accept_account)
                    .or(accounts.first())
                    .map(|(_, e)| tr_args("chat-via", &[("email", e.as_str().into())]))
                    .unwrap_or_default();
                helder::select(ui, t, &current, 360.0, |ui| {
                    for (id, email) in accounts {
                        if ui.selectable_label(app.chat.accept_account == *id, email).clicked() {
                            app.chat.accept_account = id.clone();
                            ui.close();
                        }
                    }
                });
            });
        }
        helder::icon_row(ui, t, Icon::Link, |ui| {
            let width = (ui.available_width() - 140.0).clamp(200.0, 560.0);
            let r = helder::underline_field(ui, t, &mut app.chat.accept, &tr("chat-accept-hint"), width, false);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.0);
            let ready = !app.chat.accept.trim().is_empty();
            let clicked =
                ui.add_enabled_ui(ready, |ui| helder::primary_button(ui, t, Icon::MessagesSquare, &tr("chat-accept-button"))).inner.clicked();
            if ready && (clicked || enter) {
                app.accept_invite();
            }
        });
        match &app.chat.accept_note {
            Some(Ok(note)) => {
                helder::icon_row(ui, t, Icon::CircleCheck, |ui| {
                    ui.label(RichText::new(note).color(t.text_dim));
                });
            }
            Some(Err(e)) => {
                helder::icon_row(ui, t, Icon::CircleAlert, |ui| {
                    ui.label(RichText::new(e).color(t.flag));
                });
            }
            None => {}
        }
        for p in app.engine.pending_invites() {
            let name = if p.name.is_empty() { p.addr.clone() } else { format!("{} ({})", p.name, p.addr) };
            helder::icon_row(ui, t, Icon::Clock, |ui| {
                ui.label(RichText::new(tr_args("chat-pending", &[("name", name.into())])).color(t.text_dim));
            });
        }
    });
}

/// The people in the address book that match `typed`, by name or address, leaving out who is
/// picked already.
fn suggestions(app: &MailApp, form: &NewChat) -> Vec<Address> {
    let needle = form.typed.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Address> = Vec::new();
    for c in &app.engine.session.contacts {
        for email in &c.emails {
            let matches = c.name.to_lowercase().contains(&needle) || email.to_lowercase().contains(&needle);
            let taken = form.people.iter().chain(&out).any(|p| p.email().eq_ignore_ascii_case(email));
            if matches
                && !taken
                && let Ok(a) = Address::new(Some(c.name.trim()).filter(|n| !n.is_empty()), email)
            {
                out.push(a);
            }
        }
        if out.len() >= MAX_SUGGESTIONS {
            break;
        }
    }
    out.truncate(MAX_SUGGESTIONS);
    out
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

/// One suggestion under the people field: avatar, name and address. True when clicked.
fn suggestion_row(ui: &mut Ui, t: &Tokens, person: &Address) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, person.display()));
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect, 8, t.hover);
    }
    helder::avatar(p, egui::pos2(rect.left() + 24.0, rect.center().y), 16.0, person.display(), person.email());
    let x = rect.left() + 50.0;
    p.text(egui::pos2(x, rect.top() + 7.0), egui::Align2::LEFT_TOP, person.display(), FontId::proportional(14.0), t.text);
    let line = format!("{} · {}", person.email(), tr("chat-from-contacts"));
    p.text(egui::pos2(x, rect.top() + 25.0), egui::Align2::LEFT_TOP, line, FontId::proportional(12.0), t.text_dim);
    r.clicked()
}

/// The new-chat window: the command bar (Starten, the account, Groepschat, Annuleren), the
/// people as chips with suggestions from the address book, a name for a group, and the first
/// message.
fn new_chat_form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let accounts: Vec<(String, String)> =
        app.engine.session.accounts.iter().filter(|a| engine_chat::can_chat(a)).map(|a| (a.id.clone(), a.address.email().to_string())).collect();
    let suggested = app.chat.new.as_ref().map(|f| suggestions(app, f)).unwrap_or_default();
    let mut start = false;
    let mut cancel = false;
    let ready = app.chat.new.as_ref().is_some_and(|f| app.engine.chat_ready(&f.account_id));
    let Some(form) = app.chat.new.as_mut() else { return };
    helder::quiet(ui, t, |ui| {
        ui.horizontal(|ui| {
            let can_start = !form.account_id.is_empty();
            start = ui.add_enabled_ui(can_start, |ui| helder::primary_button(ui, t, Icon::Send, &tr("chat-start"))).inner.clicked();
            helder::bar_separator(ui, t);
            let current = accounts
                .iter()
                .find(|(id, _)| *id == form.account_id)
                .map(|(_, e)| tr_args("chat-via", &[("email", e.as_str().into())]))
                .unwrap_or_else(|| tr("chat-pick-account"));
            helder::menu_button(ui, t, Some(Icon::Mail), RichText::new(current).size(14.0).color(t.text), |ui| {
                for (id, email) in &accounts {
                    if ui.selectable_label(form.account_id == *id, email).clicked() {
                        form.account_id = id.clone();
                        ui.close();
                    }
                }
            });
            helder::toggle(ui, t, &mut form.group, &tr("chat-group"));
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
                        match suggested.first().filter(|_| !comma) {
                            Some(first) => {
                                add_person(form, first.clone());
                                form.typed.clear();
                            }
                            None => {
                                commit_typed(form);
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
                            for person in &suggested {
                                if suggestion_row(ui, t, person) {
                                    add_person(form, person.clone());
                                    form.typed.clear();
                                    ui.memory_mut(|m| m.request_focus(egui::Id::new(PEOPLE)));
                                }
                            }
                        });
                }
            });
        });
        if form.group {
            helder::icon_row(ui, t, Icon::Type, |ui| {
                helder::underline_field(ui, t, &mut form.name, &tr("chat-name-hint"), width, false);
            });
        }
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
            let note = if ready { tr("chat-new-note") } else { tr("chat-key-on-start") };
            ui.add(egui::Label::new(RichText::new(note).size(12.5).color(t.text_dim)).wrap());
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
    fn typed_addresses_become_chips_and_other_text_does_not() {
        let mut form = NewChat { typed: "sanne@example.org,".into(), ..NewChat::default() };
        assert!(commit_typed(&mut form));
        assert_eq!(form.people.len(), 1);
        form.typed = "SANNE@example.org".into();
        assert!(commit_typed(&mut form));
        assert_eq!(form.people.len(), 1, "once");
        form.typed = "Sanne".into();
        assert!(!commit_typed(&mut form));
        assert!(form.error.is_some());
    }
}
