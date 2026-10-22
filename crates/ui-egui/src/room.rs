//! Merel's own meeting room: a call between Merel users, started from a chat. The engine
//! keeps who is in which room and sends the steps as encrypted chat messages; `merel_call`
//! carries the sound straight between the computers. This module joins the two each frame and
//! draws the room's window: everyone as a tile that lights up while they speak, the safety
//! code, and buttons for the microphone and for leaving.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use egui::{Align, Color32, FontId, Layout, Margin, RichText, Sense, Ui, vec2};
use merel_call::{Media, MediaEvent};
use merel_engine::CallTask;

use crate::helder::{self, Icon};
use crate::i18n::{tr, tr_args};
use crate::{MailApp, Tokens};

/// Above this level someone counts as speaking.
const SPEAKING: f32 = 0.02;

/// One connection: the room and the other person, lower case.
type Key = (String, String);

/// What the room keeps between frames.
#[derive(Default)]
pub struct State {
    /// The sound side, while this computer is in a room; started on joining, stopped on leaving.
    media: Option<Media>,
    devices: Option<merel_call::devices::Devices>,
    /// The room whose window is open.
    pub open: Option<String>,
    /// Who is loud now, by address; the account itself is "".
    levels: HashMap<String, f32>,
    /// The connections that are up.
    connected: BTreeSet<Key>,
    /// Both session descriptions of each connection, for its safety code.
    local: HashMap<Key, String>,
    remote: HashMap<Key, String>,
    /// When this computer joined each room, for the clock.
    since: HashMap<String, Instant>,
    /// The rooms that rang here already, so each rings once.
    rung: BTreeSet<String>,
    pub muted: bool,
    pub people: bool,
    pub error: Option<String>,
    /// No sound devices nor WebRTC: for the tests and screenshots, which only look at the
    /// window.
    pub no_media: bool,
}

impl State {
    /// The safety code of the connection with `peer`, once both descriptions are known.
    fn safety_code(&self, room: &str, peer: &str) -> Option<String> {
        let key = (room.to_string(), peer.to_string());
        Some(merel_call::safety_code(self.local.get(&key)?, self.remote.get(&key)?))
    }

    /// Sets how a connection stands as if it ran: for tests and screenshots.
    #[doc(hidden)]
    pub fn pretend(&mut self, room: &str, peer: &str, level: f32, sdps: Option<(String, String)>) {
        let key = (room.to_string(), peer.to_string());
        self.connected.insert(key.clone());
        self.levels.insert(peer.to_string(), level);
        if let Some((local, remote)) = sdps {
            self.local.insert(key.clone(), local);
            self.remote.insert(key, remote);
        }
        self.since.entry(room.to_string()).or_insert_with(|| Instant::now() - std::time::Duration::from_secs(754));
    }

    fn forget(&mut self, key: &Key) {
        self.connected.remove(key);
        self.local.remove(key);
        self.remote.remove(key);
        self.levels.remove(&key.1);
    }
}

/// Starts a call in the chat `chat_id` and opens the room.
pub fn start(app: &mut MailApp, chat_id: &str) {
    match app.engine.start_call(chat_id) {
        Ok(room) => {
            app.room.since.insert(room.clone(), Instant::now());
            app.room.rung.insert(room.clone());
            app.room.open = Some(room);
            app.room.error = None;
        }
        Err(e) => app.chat.error = Some(e),
    }
}

/// Joins the meeting `room` and opens it.
pub fn join(app: &mut MailApp, room: &str) {
    match app.engine.join_call(room) {
        Ok(()) => {
            app.room.since.insert(room.to_string(), Instant::now());
            app.room.open = Some(room.to_string());
            app.room.error = None;
        }
        Err(e) => app.chat.error = Some(e),
    }
}

/// Leaves `room` and closes its window.
pub fn leave(app: &mut MailApp, room: &str) {
    app.engine.leave_call(room);
    app.room.since.remove(room);
    if app.room.open.as_deref() == Some(room) {
        app.room.open = None;
    }
}

/// The meeting going on in the chat `chat_id` that this computer is not in yet, if any.
pub fn ringing_in<'a>(app: &'a MailApp, chat_id: &str) -> Option<&'a merel_engine::Call> {
    app.engine.calls().into_iter().find(|c| c.chat_id == chat_id && !c.joined && !c.present.is_empty())
}

/// The meeting in the chat `chat_id` this computer is in, if any.
pub fn joined_in<'a>(app: &'a MailApp, chat_id: &str) -> Option<&'a merel_engine::Call> {
    app.engine.calls().into_iter().find(|c| c.chat_id == chat_id && c.joined)
}

/// Runs every frame: starts or stops the sound side, hands the engine's tasks to it and its
/// offers and answers back, and says when someone rings.
pub fn tick(app: &mut MailApp, ctx: &egui::Context) {
    let in_call = app.engine.calls().iter().any(|c| c.joined);
    if in_call && app.room.media.is_none() && !app.room.no_media {
        match Media::start(vec![merel_call::DEFAULT_STUN.to_string()]) {
            Ok(media) => {
                let devices = merel_call::devices::open(media.microphone().clone(), media.mixer().clone());
                if devices.input_name.is_none() {
                    app.room.error = Some(tr("room-no-microphone"));
                }
                media.microphone().set_muted(app.room.muted);
                app.room.devices = Some(devices);
                app.room.media = Some(media);
            }
            Err(e) => app.room.error = Some(e.to_string()),
        }
    }
    for task in app.engine.take_call_tasks() {
        let media = app.room.media.as_ref();
        match task {
            CallTask::Offer { room, peer } => {
                if let Some(m) = media {
                    m.offer(&room, &peer);
                }
            }
            CallTask::Answer { room, peer, sdp } => {
                app.room.remote.insert((room.clone(), peer.clone()), sdp.clone());
                if let Some(m) = media {
                    m.answer(&room, &peer, sdp);
                }
            }
            CallTask::Accept { room, peer, sdp } => {
                app.room.remote.insert((room.clone(), peer.clone()), sdp.clone());
                if let Some(m) = media {
                    m.accept(&room, &peer, sdp);
                }
            }
            CallTask::Close { room, peer } => {
                if let Some(m) = media {
                    m.close(&room, &peer);
                }
                app.room.forget(&(room, peer));
            }
        }
    }
    let events = app.room.media.as_ref().map(Media::events).unwrap_or_default();
    for event in events {
        match event {
            MediaEvent::Offer { room, peer, sdp } => {
                app.room.local.insert((room.clone(), peer.clone()), sdp.clone());
                if let Err(e) = app.engine.send_call_offer(&room, &peer, sdp) {
                    log::warn!("call offer: {e}");
                }
            }
            MediaEvent::Answer { room, peer, sdp } => {
                app.room.local.insert((room.clone(), peer.clone()), sdp.clone());
                if let Err(e) = app.engine.send_call_answer(&room, &peer, sdp) {
                    log::warn!("call answer: {e}");
                }
            }
            MediaEvent::Connected { room, peer, connected } => {
                if connected {
                    app.room.connected.insert((room, peer));
                } else {
                    app.room.connected.remove(&(room, peer));
                }
            }
            MediaEvent::Level { peer, level } => {
                app.room.levels.insert(peer, level);
            }
            MediaEvent::Failed { room, peer, what } => {
                log::warn!("call {room} with {peer}: {what}");
                app.room.connected.remove(&(room, peer));
            }
        }
    }
    if !in_call && app.room.media.is_some() {
        app.room.devices = None;
        app.room.media = None;
        app.room.levels.clear();
        app.room.connected.clear();
        app.room.local.clear();
        app.room.remote.clear();
    }
    // Someone rings: say so once, and ask for attention.
    let ringing: Vec<(String, String)> = app
        .engine
        .calls()
        .iter()
        .filter(|c| !c.joined && !c.present.is_empty() && !app.room.rung.contains(&c.room))
        .map(|c| (c.room.clone(), c.started_by.clone()))
        .collect();
    for (room, by) in ringing {
        app.room.rung.insert(room.clone());
        let name = app
            .engine
            .call(&room)
            .and_then(|c| c.people.iter().find(|p| p.email().eq_ignore_ascii_case(&by)))
            .map(|p| p.display().to_string())
            .unwrap_or(by);
        app.notice = Some(crate::notice::Notice::info(tr_args("call-ringing", &[("name", name.into())])));
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
    }
    if in_call {
        // The speaking rings follow the sound.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// The room's window, while one is open.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(room) = app.room.open.clone() else { return };
    let Some(call) = app.engine.call(&room).cloned() else {
        app.room.open = None;
        return;
    };
    let title = app.engine.session.chats().into_iter().find(|c| c.id == call.chat_id).map(|c| c.title).unwrap_or_else(|| tr("room-title"));
    let mut open = true;
    let mut leaving = false;
    helder::item_window(ctx, t, "merel-room", &title, vec2(980.0, 640.0), &mut open, |ui| {
        leaving = room_ui(app, ui, t, &call);
    });
    if !open || leaving {
        leave(app, &room);
    }
}

/// Draws the room; returns whether Verlaten was clicked.
fn room_ui(app: &mut MailApp, ui: &mut Ui, t: &Tokens, call: &merel_engine::Call) -> bool {
    let room = call.room.as_str();
    let me = app.engine.session.accounts.iter().find(|a| a.id == call.account_id).map(|a| a.address.clone());
    // The top line: what keeps it private, how long it runs, and the safety code.
    ui.horizontal(|ui| {
        ui.add(Icon::Lock.image(15.0, t.success));
        ui.label(RichText::new(tr("chat-encrypted")).strong().size(13.0).color(t.success));
        let mut line = tr("room-direct");
        if let Some(since) = app.room.since.get(room) {
            let s = since.elapsed().as_secs();
            line = format!("{line} · {}:{:02}", s / 60, s % 60);
        }
        ui.label(RichText::new(line).size(13.0).color(t.text_dim));
        let peers: Vec<&String> = call.present.iter().collect();
        if let [only] = peers.as_slice()
            && let Some(code) = app.room.safety_code(room, only)
        {
            helder::right(ui, |ui| {
                code_label(ui, t, &code);
                ui.label(RichText::new(tr("room-safety")).size(13.0).color(t.text_dim)).on_hover_text(tr("room-safety-help"));
            });
        }
    });
    ui.add_space(8.0);
    let mut leave = false;
    ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
        ui.add_space(4.0);
        leave = controls(app, ui, t);
        ui.add_space(12.0);
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            let people_w = if app.room.people { 300.0 } else { 0.0 };
            let tiles_w = ui.available_width() - people_w - if app.room.people { 12.0 } else { 0.0 };
            let h = ui.available_height();
            ui.allocate_ui_with_layout(vec2(tiles_w, h), Layout::top_down(Align::Min), |ui| tiles(app, ui, t, call, me.as_ref(), vec2(tiles_w, h)));
            if app.room.people {
                ui.add_space(12.0);
                ui.allocate_ui_with_layout(vec2(people_w, h), Layout::top_down(Align::Min), |ui| people(app, ui, t, call, me.as_ref()));
            }
        });
    });
    leave
}

fn code_label(ui: &mut Ui, t: &Tokens, code: &str) {
    egui::Frame::new().fill(t.hover).corner_radius(6).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
        ui.label(RichText::new(code).monospace().size(13.0).color(t.text));
    });
}

/// Everyone in the room as a tile: the others who are there, then oneself.
fn tiles(app: &MailApp, ui: &mut Ui, t: &Tokens, call: &merel_engine::Call, me: Option<&merel_model::Address>, size: egui::Vec2) {
    let mut who: Vec<(String, String, String)> = call
        .present
        .iter()
        .map(|email| {
            let name =
                call.people.iter().find(|p| p.email().eq_ignore_ascii_case(email)).map(|p| p.display().to_string()).unwrap_or_else(|| email.clone());
            (name.clone(), email.clone(), name)
        })
        .collect();
    if let Some(me) = me {
        who.push((tr("room-you"), String::new(), me.display().to_string()));
    }
    let n = who.len().max(1);
    let cols = if n <= 1 {
        1
    } else if n <= 4 {
        2
    } else {
        3
    };
    let rows = n.div_ceil(cols);
    let gap = 10.0;
    let w = (size.x - gap * (cols as f32 - 1.0)) / cols as f32;
    let h = ((size.y - gap * (rows as f32 - 1.0)) / rows as f32).max(120.0);
    let origin = ui.cursor().min;
    for (i, (label, email, name)) in who.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let rect = egui::Rect::from_min_size(origin + vec2(c as f32 * (w + gap), r as f32 * (h + gap)), vec2(w, h));
        let level = app.room.levels.get(email).copied().unwrap_or(0.0);
        let mine = email.is_empty();
        let speaking = level > SPEAKING && !(mine && app.room.muted);
        let p = ui.painter();
        p.rect_filled(rect, 12.0, t.card);
        p.rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(if speaking { 3.0 } else { 1.0 }, if speaking { t.accent } else { t.card_border }),
            egui::StrokeKind::Inside,
        );
        let radius = (h * 0.18).clamp(28.0, 48.0);
        helder::avatar(p, rect.center(), radius, name, if mine { me.map(|m| m.email()).unwrap_or_default() } else { email });
        let connected = mine || app.room.connected.contains(&(call.room.clone(), email.clone()));
        let mut text = label.clone();
        if !connected {
            text = format!("{text} · {}", tr("room-connecting"));
        }
        let muted = mine && app.room.muted;
        let said = text.clone();
        ui.interact(rect, ui.id().with(("tile", i)), Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
        let galley = p.layout_no_wrap(text, FontId::proportional(13.0), Color32::WHITE);
        let icon_w = if muted { 18.0 } else { 0.0 };
        let tag = egui::Rect::from_min_size(rect.left_bottom() + vec2(10.0, -34.0), galley.size() + vec2(16.0 + icon_w, 10.0));
        p.rect_filled(tag, 6.0, Color32::from_black_alpha(190));
        if muted {
            Icon::MicOff.image(14.0, Color32::WHITE).paint_at(ui, egui::Rect::from_min_size(tag.min + vec2(8.0, 6.0), vec2(14.0, 14.0)));
        }
        ui.painter().galley(tag.min + vec2(8.0 + icon_w, 5.0), galley, Color32::WHITE);
    }
    if call.present.is_empty() {
        let at = egui::pos2(origin.x + size.x / 2.0, origin.y + 24.0);
        ui.painter().text(at, egui::Align2::CENTER_TOP, tr("room-alone"), FontId::proportional(14.0), t.text_dim);
    }
    ui.advance_cursor_after_rect(egui::Rect::from_min_size(origin, size));
}

/// The list of everyone in the chat and where they stand, with each one's safety code.
fn people(app: &mut MailApp, ui: &mut Ui, t: &Tokens, call: &merel_engine::Call, me: Option<&merel_model::Address>) {
    helder::card(ui, t, |ui| {
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            let count = call.people.len() + 1;
            ui.label(RichText::new(tr_args("room-people-count", &[("count", count.into())])).strong().size(15.0).color(t.text));
            helder::right(ui, |ui| {
                if helder::icon_button(ui, t, Icon::X, &helder::close_label()).clicked() {
                    app.room.people = false;
                }
            });
        });
        ui.add_space(8.0);
        for p in &call.people {
            let email = p.email().to_lowercase();
            let here = call.present.contains(&email);
            let state = if !here {
                tr("room-not-here")
            } else if app.room.connected.contains(&(call.room.clone(), email.clone())) {
                tr("room-connected")
            } else {
                tr("room-connecting")
            };
            person_row(ui, t, p.display(), p.email(), &state, app.room.safety_code(&call.room, &email).as_deref());
        }
        if let Some(me) = me {
            let you = format!("{} ({})", me.display(), tr("room-you").to_lowercase());
            person_row(ui, t, &you, me.email(), "", None);
        }
    });
}

fn person_row(ui: &mut Ui, t: &Tokens, name: &str, email: &str, state: &str, code: Option<&str>) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
        helder::avatar(ui.painter(), rect.center(), 16.0, name, email);
        ui.vertical(|ui| {
            ui.label(RichText::new(name).size(14.0).color(t.text));
            if !state.is_empty() {
                ui.label(RichText::new(state).size(12.0).color(t.text_dim));
            }
            if let Some(code) = code {
                ui.label(RichText::new(code).monospace().size(12.0).color(t.text_dim)).on_hover_text(tr("room-safety-help"));
            }
        });
    });
    ui.add_space(6.0);
}

/// The round buttons under the room; returns whether Verlaten was clicked.
fn controls(app: &mut MailApp, ui: &mut Ui, t: &Tokens) -> bool {
    let mut leave = false;
    let width = 3.0 * 96.0 + 2.0 * 16.0 + 32.0;
    ui.allocate_ui_with_layout(vec2(width, 80.0), Layout::left_to_right(Align::Min), |ui| {
        let muted = app.room.muted;
        let mic = if muted { Icon::MicOff } else { Icon::Mic };
        if round(ui, t, mic, &tr("room-microphone"), !muted, false).clicked() {
            app.room.muted = !muted;
            if let Some(m) = &app.room.media {
                m.microphone().set_muted(app.room.muted);
            }
        }
        ui.add_space(16.0);
        if round(ui, t, Icon::Users, &tr("room-people"), app.room.people, false).clicked() {
            app.room.people = !app.room.people;
        }
        ui.add_space(48.0);
        leave = round(ui, t, Icon::PhoneOff, &tr("room-leave"), false, true).clicked();
    });
    if let Some(e) = &app.room.error {
        ui.label(RichText::new(e).size(12.5).color(t.flag));
    }
    leave
}

/// A round button with its name under it, as in the design: filled in the selection colour
/// when on, red for leaving.
fn round(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, on: bool, danger: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(96.0, 72.0), Sense::click());
    response.widget_info(|| {
        if danger {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
        } else {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, label)
        }
    });
    let center = egui::pos2(rect.center().x, rect.top() + 26.0);
    let (fill, stroke, color) = if danger {
        (t.flag, Color32::TRANSPARENT, Color32::WHITE)
    } else if on {
        (t.checked, Color32::TRANSPARENT, t.accent_text)
    } else {
        (t.card, t.control_border, t.text)
    };
    let p = ui.painter();
    let fill = if response.hovered() { fill.gamma_multiply(0.9) } else { fill };
    p.circle(center, 26.0, fill, egui::Stroke::new(1.0, stroke));
    icon.image(20.0, color).paint_at(ui, egui::Rect::from_center_size(center, vec2(20.0, 20.0)));
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 12.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        if danger { t.flag } else { t.text },
    );
    response.on_hover_text(label)
}
