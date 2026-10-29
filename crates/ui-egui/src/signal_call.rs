//! Calls over Signal: the program `merel-signal` plays and records the sound itself with
//! Signal's own calling library, so this module only shows the call: who calls, answer or
//! decline, how long it runs, the microphone and hanging up. Signal encrypts the call end to end,
//! as it does in its own apps.

use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, Ui, vec2};
use merel_engine::signal::SignalCallPhase as Phase;

use crate::helder::{self, Icon};
use crate::i18n::{tr, tr_args};
use crate::{MailApp, Tokens};

/// How long the window stays after a call ended, to say how.
const LINGER: Duration = Duration::from_secs(3);

/// The call shown, if any.
#[derive(Default)]
pub struct State {
    pub call: Option<Call>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Call {
    pub account_id: String,
    /// The other person's Signal address.
    pub with: String,
    pub name: String,
    pub call_id: u64,
    pub phase: Phase,
    /// Since when it is connected, for the clock.
    pub since: Option<Instant>,
    /// Since when it is over.
    pub over: Option<Instant>,
    pub muted: bool,
}

/// Whether the chat with `with` on `account_id` can be called over Signal from here.
pub fn can_call(app: &MailApp, account_id: &str) -> bool {
    app.engine.signal_can_call(account_id)
}

/// Calls `with` over Signal and opens the call's window.
pub fn start(app: &mut MailApp, account_id: &str, with: &str, name: &str) {
    if app.signal_call.call.as_ref().is_some_and(|c| c.over.is_none()) {
        return;
    }
    match app.engine.signal_call(account_id, with) {
        Ok(()) => {
            app.signal_call.error = None;
            // The id comes with the first news from the program.
            app.signal_call.call = Some(Call {
                account_id: account_id.to_string(),
                with: with.to_lowercase(),
                name: name.to_string(),
                call_id: 0,
                phase: Phase::Outgoing,
                since: None,
                over: None,
                muted: false,
            });
        }
        Err(e) => app.chat.error = Some(e),
    }
}

/// Runs every frame: takes in how calls go, and rings for an incoming one.
pub fn tick(app: &mut MailApp, ctx: &egui::Context) {
    for (account_id, update) in app.engine.take_signal_calls() {
        let mine = app.signal_call.call.as_ref().is_some_and(|c| c.call_id == update.call_id || (c.call_id == 0 && c.with == update.thread));
        let busy = app.signal_call.call.as_ref().is_some_and(|c| c.over.is_none());
        if !mine && busy {
            // Another call while one goes on: Signal tells the caller this one is busy.
            continue;
        }
        if !mine {
            if update.state != Phase::Incoming {
                continue;
            }
            let name = if update.name.is_empty() { update.thread.clone() } else { update.name.clone() };
            app.notice = Some(crate::notice::Notice::info(tr_args("signal-call-incoming", &[("name", name.clone().into())])));
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
            app.signal_call.error = None;
            app.signal_call.call = Some(Call {
                account_id,
                with: update.thread.clone(),
                name,
                call_id: update.call_id,
                phase: Phase::Incoming,
                since: None,
                over: None,
                muted: false,
            });
            continue;
        }
        if let Some(call) = &mut app.signal_call.call {
            call.call_id = update.call_id;
            // The program says "outgoing" or "incoming" once more as it goes on; that changes nothing.
            if !matches!(update.state, Phase::Incoming | Phase::Outgoing) {
                call.phase = update.state;
            }
            if update.state == Phase::Connected && call.since.is_none() {
                call.since = Some(Instant::now());
            }
            if update.state.is_over() && call.over.is_none() {
                call.over = Some(Instant::now());
            }
        }
    }
    if let Some(call) = &app.signal_call.call {
        match call.over {
            Some(over) if over.elapsed() > LINGER => app.signal_call.call = None,
            Some(_) => ctx.request_repaint_after(Duration::from_millis(500)),
            // The clock ticks.
            None => ctx.request_repaint_after(Duration::from_secs(1)),
        }
    }
}

/// What the window says about where the call stands.
fn status(call: &Call) -> String {
    match call.phase {
        Phase::Incoming => tr("signal-call-state-incoming"),
        Phase::Outgoing | Phase::Connecting => tr("signal-call-state-connecting"),
        Phase::Ringing => tr("signal-call-state-ringing"),
        Phase::Connected => {
            let s = call.since.map(|s| s.elapsed().as_secs()).unwrap_or_default();
            format!("{}:{:02}", s / 60, s % 60)
        }
        Phase::AnsweredElsewhere => tr("signal-call-state-elsewhere"),
        Phase::Declined => tr("signal-call-state-declined"),
        Phase::Ended => tr("signal-call-state-ended"),
        Phase::Failed => tr("signal-call-state-failed"),
    }
}

/// The call's window, while there is one.
pub fn window(app: &mut MailApp, ctx: &egui::Context, t: &Tokens) {
    let Some(call) = app.signal_call.call.clone() else { return };
    let mut open = true;
    let mut hang_up = false;
    helder::item_window(ctx, t, "merel-signal-call", &tr("signal-call-title"), vec2(380.0, 400.0), &mut open, |ui| {
        hang_up = call_ui(app, ui, t, &call);
    });
    if (!open || hang_up) && call.over.is_none() {
        if let Err(e) = app.engine.signal_hang_up(&call.account_id) {
            app.signal_call.error = Some(e);
        }
        if let Some(c) = &mut app.signal_call.call {
            c.phase = Phase::Ended;
            c.over = Some(Instant::now());
        }
    }
    if !open {
        app.signal_call.call = None;
    }
}

/// Draws the call; returns whether it was hung up or declined.
fn call_ui(app: &mut MailApp, ui: &mut Ui, t: &Tokens, call: &Call) -> bool {
    let mut hang_up = false;
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        let (rect, _) = ui.allocate_exact_size(vec2(96.0, 96.0), egui::Sense::hover());
        helder::avatar(ui.painter(), rect.center(), 48.0, &call.name, &call.with);
        ui.add_space(12.0);
        ui.label(RichText::new(&call.name).strong().size(20.0).color(t.text));
        ui.add_space(4.0);
        ui.label(RichText::new(status(call)).size(14.0).color(t.text_dim));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let w = 15.0 + 6.0 + ui.painter().layout_no_wrap(tr("signal-call-encrypted"), egui::FontId::proportional(13.0), t.success).size().x;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            ui.add(Icon::Lock.image(15.0, t.success));
            ui.label(RichText::new(tr("signal-call-encrypted")).size(13.0).color(t.success));
        });
    });
    ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
        ui.add_space(12.0);
        if let Some(e) = &app.signal_call.error {
            ui.label(RichText::new(e).size(12.5).color(t.flag));
        }
        if call.over.is_some() {
            return;
        }
        let incoming = call.phase == Phase::Incoming;
        let width = 2.0 * 96.0 + 48.0;
        ui.allocate_ui_with_layout(vec2(width, 80.0), Layout::left_to_right(Align::Min), |ui| {
            if incoming {
                if crate::room::round(ui, t, Icon::Phone, &tr("signal-call-answer"), true, false).clicked() {
                    match app.engine.signal_accept(&call.account_id, call.call_id) {
                        Ok(()) => {
                            if let Some(c) = &mut app.signal_call.call {
                                c.phase = Phase::Connecting;
                            }
                        }
                        Err(e) => app.signal_call.error = Some(e),
                    }
                }
                ui.add_space(48.0);
                hang_up = crate::room::round(ui, t, Icon::PhoneOff, &tr("signal-call-decline"), false, true).clicked();
            } else {
                let mic = if call.muted { Icon::MicOff } else { Icon::Mic };
                if crate::room::round(ui, t, mic, &tr("room-microphone"), !call.muted, false).clicked() {
                    let muted = !call.muted;
                    match app.engine.signal_mute(&call.account_id, muted) {
                        Ok(()) => {
                            if let Some(c) = &mut app.signal_call.call {
                                c.muted = muted;
                            }
                        }
                        Err(e) => app.signal_call.error = Some(e),
                    }
                }
                ui.add_space(48.0);
                hang_up = crate::room::round(ui, t, Icon::PhoneOff, &tr("signal-call-hang-up"), false, true).clicked();
            }
        });
    });
    hang_up
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn ringing() -> Call {
        Call {
            account_id: "signal-1".into(),
            with: "1b2c@signal.invalid".into(),
            name: "Jan de Vries".into(),
            call_id: 7,
            phase: Phase::Incoming,
            since: None,
            over: None,
            muted: false,
        }
    }

    #[test]
    fn an_incoming_call_offers_to_answer_or_decline() {
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.signal_call.call = Some(ringing());
        let mut harness = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        assert!(harness.query_by_label("Opnemen").is_some());
        assert!(harness.query_by_label("Ophangen").is_none());
        // No program runs here, so declining cannot reach Signal; the window still ends the call.
        harness.get_by_label("Weigeren").click();
        harness.run();
        let call = harness.state().signal_call.call.clone();
        assert!(call.is_some_and(|c| c.over.is_some() && c.phase == Phase::Ended));
        assert!(harness.query_by_label("Weigeren").is_none());
    }

    #[test]
    fn a_call_going_on_shows_its_clock_and_can_be_muted_and_hung_up() {
        let mut call = ringing();
        call.phase = Phase::Connected;
        call.since = Some(Instant::now() - Duration::from_secs(75));
        assert_eq!(status(&call), "1:15");
        let mut app = MailApp::new(merel_engine::Engine::sample());
        app.signal_call.call = Some(call);
        let mut harness = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        harness.run();
        assert!(harness.query_by_label("Ophangen").is_some());
        assert!(harness.query_by_label("Microfoon").is_some());
        assert!(harness.query_by_label("Opnemen").is_none());
    }
}
