//! Voice messages in a chat, as Signal shows them: a round button to play and pause, a bar
//! that shows how far it is (click or drag it to jump), and the time. They play in Merel
//! itself; one that cannot be decoded opens in the computer's own player.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use egui::{RichText, Sense, Ui, vec2};

use crate::Tokens;
use crate::helder::Icon;
use crate::i18n::tr;

/// A voice message: the message id and which of its attachments.
type Key = (String, usize);

/// The voice messages heard in this chat window and the one playing.
#[derive(Default)]
pub struct Player {
    /// Decoded voice messages, or why one could not be; decoded the first time one is drawn.
    clips: RefCell<HashMap<Key, Result<merel_voice::Clip, String>>>,
    /// The one playing or paused.
    current: Option<Current>,
}

struct Current {
    key: Key,
    /// How far it was when paused, from 0 to 1.
    at: f32,
    playing: Option<merel_voice::Playback>,
}

/// What a click on a voice message asks.
pub enum Ask {
    /// Play it, or pause it when it plays.
    Toggle,
    /// Jump to this point, from 0 to 1.
    Seek(f32),
}

impl Player {
    fn clip(&self, key: &Key, a: &merel_mime::Attachment) -> Result<merel_voice::Clip, String> {
        self.clips.borrow_mut().entry(key.clone()).or_insert_with(|| merel_voice::decode(&a.bytes, &a.mime)).clone()
    }

    /// Whether `key` plays now, and how far it is.
    fn state(&self, key: &Key) -> (bool, f32) {
        match &self.current {
            Some(c) if c.key == *key => match &c.playing {
                Some(p) => (true, p.progress()),
                None => (false, c.at),
            },
            _ => (false, 0.0),
        }
    }

    /// Does what was asked for attachment `n` of message `id`. A voice message Merel cannot
    /// decode opens in the computer's own player.
    pub fn ask(&mut self, id: &str, n: usize, a: &merel_mime::Attachment, ask: Ask) -> Result<(), String> {
        let key = (id.to_string(), n);
        let clip = match self.clip(&key, a) {
            Ok(clip) => clip,
            Err(e) => {
                log::info!("voice message {id}/{n} plays outside Merel: {e}");
                return crate::attachments::open_in_app(a);
            }
        };
        let (playing, at) = self.state(&key);
        let (play, from) = match ask {
            Ask::Toggle if playing => (false, at),
            Ask::Toggle => (true, if at >= 0.999 { 0.0 } else { at }),
            Ask::Seek(to) => (playing, to.clamp(0.0, 1.0)),
        };
        // The one playing stops first: one voice at a time.
        self.current = None;
        let playing = if play { Some(merel_voice::play(&clip, from)?) } else { None };
        self.current = Some(Current { key, at: from, playing });
        Ok(())
    }

    /// Each frame: lets go of one that has ended, and redraws the bar of one that plays.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.current else { return };
        match &c.playing {
            Some(p) if p.finished() => self.current = None,
            Some(_) => ctx.request_repaint_after(Duration::from_millis(50)),
            None => {}
        }
    }

    /// Stops what plays, as when the chat closes.
    pub fn stop(&mut self) {
        self.current = None;
    }
}

/// `d` as Signal writes it: 0:07, 1:23.
fn time(d: Duration) -> String {
    let s = (d.as_millis() + 500) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The row for voice message `n` of message `id`.
pub fn row(ui: &mut Ui, t: &Tokens, player: &Player, id: &str, n: usize, a: &merel_mime::Attachment) -> Option<Ask> {
    let key = (id.to_string(), n);
    let clip = player.clip(&key, a);
    let (playing, at) = player.state(&key);
    let mut ask = None;
    ui.horizontal(|ui| {
        let (rect, r) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
        ui.painter().circle_filled(rect.center(), 16.0, t.accent);
        let icon = if playing { Icon::Pause } else { Icon::Play };
        icon.image(16.0, t.on_accent).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(16.0, 16.0)));
        let label = tr(if playing { "chat-voice-pause" } else { "chat-voice-play" });
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(label).clicked() {
            ask = Some(Ask::Toggle);
        }
        ui.add_space(4.0);
        match &clip {
            Ok(clip) => {
                let length = clip.duration();
                let (bar, r) = ui.allocate_exact_size(vec2(160.0, 32.0), Sense::click_and_drag());
                let y = bar.center().y;
                let x = bar.left() + bar.width() * at;
                let p = ui.painter();
                p.hline(bar.left()..=bar.right(), y, egui::Stroke::new(3.0, t.text_dim.gamma_multiply(0.5)));
                p.hline(bar.left()..=x, y, egui::Stroke::new(3.0, t.accent));
                p.circle_filled(egui::pos2(x, y), 6.0, t.accent);
                let shown = if playing || at > 0.0 { length.mul_f32(at) } else { length };
                let said = format!("{}, {} / {}", tr("chat-voice"), time(length.mul_f32(at)), time(length));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, said.clone()));
                if (r.clicked() || r.drag_stopped())
                    && let Some(pos) = r.interact_pointer_pos()
                {
                    ask = Some(Ask::Seek((pos.x - bar.left()) / bar.width()));
                }
                ui.add_space(8.0);
                ui.label(RichText::new(time(shown)).size(13.0).color(t.text_dim));
            }
            // As before: what it is and how big; the button opens it elsewhere.
            Err(_) => {
                ui.add(Icon::Mic.image(14.0, t.text_dim));
                ui.label(RichText::new(format!("{}  ({})", tr("chat-voice"), crate::size_text(a.size))).size(13.0).color(t.text));
            }
        }
    });
    ask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_in_signal() {
        assert_eq!(time(Duration::from_millis(7400)), "0:07");
        assert_eq!(time(Duration::from_millis(2990)), "0:03");
        assert_eq!(time(Duration::from_secs(83)), "1:23");
    }
}
