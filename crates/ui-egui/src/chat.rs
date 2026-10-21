//! The chat module: the chats on the left, the open one on the right with its messages and a
//! line to write, and the form for a new chat. Chat goes over e-mail, Delta Chat's way, and
//! only end-to-end encrypted; see `merel_engine::chat`.

use chrono::Local;
use egui::{Align, FontId, Layout, Margin, RichText, Sense, Ui, vec2};

use merel_engine::chat::{self as engine_chat, Chat};
use merel_engine::{SAMPLE_ACCOUNT, SendState};
use merel_model::{Address, parse_address_list};

use crate::calendar::to_local;
use crate::helder::{self, Icon};
use crate::i18n::{tr, tr_args};
use crate::{MailApp, Tokens};

/// The search box's widget id, so Ctrl+E can put the cursor there.
pub(crate) const SEARCH_BOX: &str = "chat_search_box";
/// The line to write in: its widget id.
pub(crate) const INPUT: &str = "chat-input";

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
}

/// The form for a new chat.
#[derive(Default)]
pub struct NewChat {
    /// The account to chat from.
    pub account_id: String,
    /// The addresses, as typed.
    pub to: String,
    /// A name, which makes it a group.
    pub name: String,
    pub text: String,
    pub error: Option<String>,
}

impl MailApp {
    /// Opens the form for a new chat, from the first account that can chat.
    pub fn new_chat(&mut self) {
        let account_id = self.chat_accounts().into_iter().find(|id| self.engine.chat_ready(id)).or_else(|| self.chat_accounts().into_iter().next());
        self.chat.new = Some(NewChat { account_id: account_id.unwrap_or_default(), ..NewChat::default() });
        self.chat.error = None;
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

    /// Sends what is typed in the open chat.
    fn send_in_chat(&mut self) {
        let Some(chat) = self.chat.selected.as_deref().and_then(|id| self.engine.session.chat(id)) else { return };
        let text = self.chat.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let draft = self.engine.session.chat_reply(&chat, &text);
        match self.engine.send_chat(&chat.account_id, draft) {
            Ok(id) => {
                self.chat.sending = Some(id);
                self.chat.input.clear();
                self.chat.error = None;
            }
            Err(e) => self.chat.error = Some(e),
        }
    }

    /// Starts the chat the form describes.
    fn start_chat(&mut self) {
        let Some(form) = self.chat.new.as_mut() else { return };
        let people = match parse_address_list(&form.to) {
            Ok(p) => p,
            Err(_) => {
                form.error = Some(tr("chat-check-addresses"));
                return;
            }
        };
        let draft = match engine_chat::new_chat(&people, &form.name, &form.text) {
            Ok(d) => d,
            Err(e) => {
                form.error = Some(e);
                return;
            }
        };
        let account_id = form.account_id.clone();
        match self.engine.send_chat(&account_id, draft) {
            Ok(id) => {
                self.chat.sending = Some(id);
                self.chat.new = None;
            }
            Err(e) => {
                if let Some(form) = self.chat.new.as_mut() {
                    form.error = Some(e);
                }
            }
        }
    }
}

/// The chats the list shows, by the filter and the search box.
fn listed(app: &MailApp) -> Vec<Chat> {
    let needle = app.chat.query.trim().to_lowercase();
    app.engine
        .session
        .chats()
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

pub fn show(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    list_pane(app, ui, t);
    let frame = helder::card_frame(t).inner_margin(Margin::same(20)).outer_margin(Margin { left: 6, right: 12, top: 0, bottom: 12 });
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        if app.chat.new.is_some() {
            new_chat_form(app, ui, t);
        } else if let Some(chat) = app.chat.selected.as_deref().and_then(|id| app.engine.session.chat(id)) {
            conversation(app, ui, t, &chat);
        } else {
            start_page(app, ui, t);
        }
    });
}

fn list_pane(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("chat-list")
        .default_size(340.0)
        .resizable(true)
        .show_separator_line(false)
        .frame(helder::card_frame(t).inner_margin(Margin::same(12)).outer_margin(Margin { left: 0, right: 6, top: 0, bottom: 12 }))
        .show(ui, |ui| {
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

/// One chat in the list: avatar, name, when, the last message, and how many are unread.
fn chat_row(app: &MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) -> egui::Response {
    let on = app.chat.selected.as_deref() == Some(chat.id.as_str());
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
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
    let first = chat.people.first();
    let avatar_name = if chat.is_group() { chat.title.as_str() } else { first.map(Address::display).unwrap_or("") };
    let avatar_mail = if chat.is_group() { chat.id.as_str() } else { first.map(Address::email).unwrap_or("") };
    helder::avatar(p, egui::pos2(rect.left() + 24.0, rect.center().y), 16.0, avatar_name, avatar_mail);
    let x = rect.left() + 50.0;
    let right = rect.right() - 8.0;
    let stamp = when(chat.last);
    let stamp_w = p.layout_no_wrap(stamp.clone(), FontId::proportional(12.0), t.text_dim).size().x;
    p.text(egui::pos2(right, rect.top() + 10.0), egui::Align2::RIGHT_TOP, stamp, FontId::proportional(12.0), t.text_dim);
    let unread = chat.unread > 0;
    let name_color = if unread { t.accent_text } else { t.text };
    let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(right - stamp_w - 8.0, rect.bottom()));
    p.with_clip_rect(clip).text(egui::pos2(x, rect.top() + 9.0), egui::Align2::LEFT_TOP, &chat.title, FontId::proportional(14.0), name_color);
    let last = chat.messages.last().map(String::as_str).unwrap_or("");
    let preview = app.engine.session.chat_text(last).map(|s| s.replace('\n', " ")).unwrap_or_else(|| format!("🔒 {}", tr("chat-encrypted")));
    let preview = if app.engine.session.chat_is_mine(last) { tr_args("chat-mine", &[("text", preview.as_str().into())]) } else { preview };
    let badge_w = if unread { 26.0 } else { 0.0 };
    let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(right - badge_w, rect.bottom()));
    let preview_color = if unread { t.text } else { t.text_dim };
    p.with_clip_rect(clip).text(egui::pos2(x, rect.top() + 30.0), egui::Align2::LEFT_TOP, preview, FontId::proportional(13.0), preview_color);
    if unread {
        let c = egui::pos2(right - 10.0, rect.top() + 38.0);
        p.circle_filled(c, 9.0, t.accent);
        p.text(c, egui::Align2::CENTER_CENTER, chat.unread.min(99).to_string(), FontId::proportional(11.0), t.on_accent);
    }
    r
}

/// The open chat: its name, the messages oldest first, and the line to write.
fn conversation(app: &mut MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        let first = chat.people.first();
        let (name, mail) = if chat.is_group() {
            (chat.title.as_str(), chat.id.as_str())
        } else {
            (first.map(Address::display).unwrap_or(""), first.map(Address::email).unwrap_or(""))
        };
        helder::avatar(ui.painter(), rect.center(), 20.0, name, mail);
        ui.vertical(|ui| {
            ui.label(RichText::new(&chat.title).size(17.0).color(t.text));
            let people = chat.people.iter().map(Address::email).collect::<Vec<_>>().join(", ");
            ui.horizontal(|ui| {
                ui.add(Icon::Lock.image(14.0, t.text_dim));
                ui.label(RichText::new(format!("{} · {people}", tr("chat-encrypted"))).size(12.5).color(t.text_dim));
            });
        });
    });
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, t.card_border));
    ui.add_space(8.0);
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
            egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
                for id in &chat.messages {
                    bubble(app, ui, t, chat, id);
                }
            });
        });
    });
}

/// One message: on the right in the accent's tint when it is one's own, else on the left
/// with the sender's name over it in a group.
fn bubble(app: &MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat, id: &str) {
    let session = &app.engine.session;
    let Some(m) = session.message(id) else { return };
    let mine = session.chat_is_mine(id);
    let text = session.chat_text(id).unwrap_or_else(|| tr("chat-loading"));
    let max = (ui.available_width() * 0.7).min(460.0);
    // Your own messages line up on the right, the others' on the left, as in any chat.
    let layout = Layout::top_down(if mine { Align::Max } else { Align::Min });
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 0.0), layout, |ui| {
        let who = if mine || !chat.is_group() { when(m.date) } else { format!("{} · {}", m.from.display(), when(m.date)) };
        ui.label(RichText::new(who).size(12.0).color(t.text_dim));
        egui::Frame::new().fill(if mine { t.checked } else { t.window }).corner_radius(12).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_max_width(max);
            ui.add(egui::Label::new(RichText::new(text).size(14.0).color(t.text)).wrap());
        });
    });
    ui.add_space(8.0);
}

fn composer(app: &mut MailApp, ui: &mut Ui, t: &Tokens, chat: &Chat) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 130.0).max(120.0);
        let hint = tr_args("chat-input-hint", &[("name", chat.title.as_str().into())]);
        let r = ui.add(
            egui::TextEdit::singleline(&mut app.chat.input)
                .id(egui::Id::new(INPUT))
                .hint_text(RichText::new(hint).color(t.text_dim))
                .desired_width(width)
                .margin(Margin::symmetric(10, 8)),
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, tr("chat-input")));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let ready = !app.chat.input.trim().is_empty();
        let clicked = ui.add_enabled_ui(ready, |ui| helder::primary_button(ui, t, Icon::Send, &tr("chat-send"))).inner.clicked();
        if (enter || clicked) && ready {
            app.send_in_chat();
            ui.memory_mut(|m| m.request_focus(egui::Id::new(INPUT)));
        }
    });
}

/// What the right side shows without an open chat: how to start, and chat to turn on.
fn start_page(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    ui.label(RichText::new(tr("chat-title")).size(20.0).color(t.text));
    ui.add_space(8.0);
    helder::paragraph(ui, t, &tr("chat-intro"));
    ui.add_space(12.0);
    let accounts: Vec<(String, String)> =
        app.engine.session.accounts.iter().filter(|a| engine_chat::can_chat(a)).map(|a| (a.id.clone(), a.address.email().to_string())).collect();
    if accounts.is_empty() {
        let key = if app.engine.session.accounts.iter().all(|a| a.id == SAMPLE_ACCOUNT) { "chat-add-account" } else { "chat-imap-only" };
        helder::note(ui, t, &tr(key));
        return;
    }
    for (id, email) in accounts {
        helder::icon_row(ui, t, Icon::Lock, |ui| {
            if app.engine.chat_ready(&id) {
                ui.label(RichText::new(tr_args("chat-on", &[("email", email.as_str().into())])).color(t.text));
            } else {
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr_args("chat-off", &[("email", email.as_str().into())])).color(t.text));
                    ui.label(RichText::new(tr("chat-key-note")).size(12.5).color(t.text_dim));
                    if helder::secondary(ui, t, |ui| ui.button(tr_args("chat-enable", &[("email", email.as_str().into())]))).clicked() {
                        match app.engine.enable_chat(&id) {
                            Ok(_) => app.chat.error = None,
                            Err(e) => app.chat.error = Some(e),
                        }
                    }
                });
            }
        });
        ui.add_space(6.0);
    }
    if let Some(e) = &app.chat.error {
        ui.label(RichText::new(e).color(t.flag));
    }
    ui.add_space(12.0);
    if helder::primary_button(ui, t, Icon::MessagesSquare, &tr("chat-new")).clicked() {
        app.new_chat();
    }
}

/// The form for a new chat: from which account, with whom, a name for a group, and the first
/// message.
fn new_chat_form(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let accounts: Vec<(String, String)> =
        app.engine.session.accounts.iter().filter(|a| engine_chat::can_chat(a)).map(|a| (a.id.clone(), a.address.email().to_string())).collect();
    let mut start = false;
    let mut cancel = false;
    let ready = app.chat.new.as_ref().is_some_and(|f| app.engine.chat_ready(&f.account_id));
    ui.horizontal(|ui| {
        start = ui.add_enabled_ui(ready, |ui| helder::primary_button(ui, t, Icon::Send, &tr("chat-start"))).inner.clicked();
        helder::trailing(ui, |ui| {
            cancel = helder::quiet(ui, t, |ui| helder::tool_button(ui, t, Icon::X, &tr("chat-cancel"))).clicked();
        });
    });
    ui.add_space(12.0);
    let Some(form) = app.chat.new.as_mut() else { return };
    let width = (ui.available_width() - 40.0).max(200.0);
    helder::icon_row(ui, t, Icon::Mail, |ui| {
        let current = accounts
            .iter()
            .find(|(id, _)| *id == form.account_id)
            .map(|(_, e)| tr_args("chat-via", &[("email", e.as_str().into())]))
            .unwrap_or_else(|| tr("chat-pick-account"));
        helder::select(ui, t, &current, width.min(360.0), |ui| {
            for (id, email) in &accounts {
                if ui.selectable_label(form.account_id == *id, email).clicked() {
                    form.account_id = id.clone();
                    ui.close();
                }
            }
        });
    });
    helder::icon_row(ui, t, Icon::Users, |ui| {
        helder::underline_field(ui, t, &mut form.to, &tr("chat-to-hint"), width, false);
    });
    helder::icon_row(ui, t, Icon::Type, |ui| {
        helder::underline_field(ui, t, &mut form.name, &tr("chat-name-hint"), width, false);
    });
    helder::icon_row(ui, t, Icon::Notes, |ui| {
        ui.add(egui::TextEdit::multiline(&mut form.text).hint_text(tr("chat-first-hint")).desired_width(width).desired_rows(5));
    });
    if let Some(e) = &form.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.flag));
    }
    ui.add_space(10.0);
    helder::note(ui, t, &tr(if ready { "chat-ready-note" } else { "chat-enable-first" }));
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
    if crate::ribbon_action(ui, t, "◎", &tr("chat-all"), open || app.chat.new.is_some()) {
        app.chat.selected = None;
        app.chat.new = None;
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
}
