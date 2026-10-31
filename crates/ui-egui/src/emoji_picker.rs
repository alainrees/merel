//! The emoji picker, as Signal has it: a field to search (Dutch and English words), the emoji
//! used lately, then every emoji in Signal's groups, with a row of tabs under it to jump to a
//! group. Only the rows in sight are drawn, so the whole set stays quick.
//!
//! The emoji, their order and their words come from `assets/emoji/picker.txt`, made by
//! `packaging/tools/make-emoji-picker.py` from Google's emoji metadata and Unicode's CLDR.

use std::sync::OnceLock;

use egui::{Rect, RichText, Sense, Ui, Vec2, vec2};

use crate::helder::{self, Icon};
use crate::i18n::tr;
use crate::theme::Tokens;

const DATA: &str = include_str!("../assets/emoji/picker.txt");

/// Emoji in a row, as in Signal.
const COLUMNS: usize = 8;
/// The square each emoji takes, and the picture in it.
const CELL: f32 = 38.0;
const PICTURE: f32 = 26.0;
const HEADING: f32 = 30.0;
/// How tall the list is; it scrolls.
const HEIGHT: f32 = 320.0;
/// How many emoji "Onlangs gebruikt" keeps.
pub const RECENT: usize = 32;

/// Signal's groups, in its order, each with the emoji its tab shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Group {
    People,
    Animals,
    Food,
    Activities,
    Travel,
    Objects,
    Symbols,
    Flags,
}

impl Group {
    const ALL: [Group; 8] =
        [Group::People, Group::Animals, Group::Food, Group::Activities, Group::Travel, Group::Objects, Group::Symbols, Group::Flags];

    fn from_key(key: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.key() == key)
    }

    fn key(self) -> &'static str {
        match self {
            Group::People => "people",
            Group::Animals => "animals",
            Group::Food => "food",
            Group::Activities => "activities",
            Group::Travel => "travel",
            Group::Objects => "objects",
            Group::Symbols => "symbols",
            Group::Flags => "flags",
        }
    }

    fn name(self) -> String {
        tr(&format!("emoji-group-{}", self.key()))
    }

    fn tab(self) -> &'static str {
        match self {
            Group::People => "😀",
            Group::Animals => "🐻",
            Group::Food => "🍜",
            Group::Activities => "⚽",
            Group::Travel => "🚗",
            Group::Objects => "💡",
            Group::Symbols => "🔣",
            Group::Flags => "🏳️",
        }
    }
}

/// One emoji and the words it is found by, lower case.
pub(crate) struct Entry {
    pub text: &'static str,
    words: &'static str,
}

impl Entry {
    /// Whether every word of `query` starts one of this emoji's words.
    fn matches(&self, query: &[String]) -> bool {
        query.iter().all(|q| self.words.split(' ').any(|w| w.starts_with(q.as_str())))
    }
}

/// Every emoji, by group.
pub(crate) fn catalog() -> &'static [(Group, Vec<Entry>)] {
    static CATALOG: OnceLock<Vec<(Group, Vec<Entry>)>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut out: Vec<(Group, Vec<Entry>)> = Vec::new();
        for line in DATA.lines() {
            if let Some(key) = line.strip_prefix('@') {
                if let Some(g) = Group::from_key(key) {
                    out.push((g, Vec::new()));
                }
            } else if let (Some((text, words)), Some((_, list))) = (line.split_once('\t'), out.last_mut()) {
                list.push(Entry { text, words });
            }
        }
        out
    })
}

/// The emoji used lately, newest first; Merel keeps them with the view choices.
pub fn recent(ctx: &egui::Context) -> Vec<String> {
    ctx.data(|d| d.get_temp::<Vec<String>>(egui::Id::new(RECENT_ID))).unwrap_or_default()
}

pub fn set_recent(ctx: &egui::Context, list: Vec<String>) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(RECENT_ID), list));
}

const RECENT_ID: &str = "merel-emoji-recent";

/// Puts `emoji` first among the ones used lately.
pub fn remember(ctx: &egui::Context, emoji: &str) {
    let mut list = recent(ctx);
    list.retain(|e| e != emoji);
    list.insert(0, emoji.to_string());
    list.truncate(RECENT);
    set_recent(ctx, list);
}

/// What one picker holds between frames.
#[derive(Clone, Default)]
struct State {
    query: String,
    /// The group to scroll to on the next frame (`None` inside: the emoji used lately).
    jump: Option<Option<Group>>,
    /// The group at the top of the list, whose tab is lit.
    shown: Option<Group>,
}

/// A row of the list: a heading, or up to [`COLUMNS`] emoji.
enum Row<'a> {
    Heading(String, Option<Group>),
    Emoji(Vec<&'a str>),
}

impl Row<'_> {
    fn height(&self) -> f32 {
        match self {
            Row::Heading(..) => HEADING,
            Row::Emoji(_) => CELL,
        }
    }
}

fn rows<'a>(recent: &'a [String], query: &str) -> Vec<Row<'a>> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    let mut out = Vec::new();
    let add = |out: &mut Vec<Row<'a>>, emoji: Vec<&'a str>| {
        for chunk in emoji.chunks(COLUMNS) {
            out.push(Row::Emoji(chunk.to_vec()));
        }
    };
    if !words.is_empty() {
        let found: Vec<&str> = catalog().iter().flat_map(|(_, list)| list).filter(|e| e.matches(&words)).map(|e| e.text).collect();
        add(&mut out, found);
        return out;
    }
    if !recent.is_empty() {
        out.push(Row::Heading(tr("emoji-recent"), None));
        add(&mut out, recent.iter().map(String::as_str).collect());
    }
    for (group, list) in catalog() {
        out.push(Row::Heading(group.name(), Some(*group)));
        add(&mut out, list.iter().map(|e| e.text).collect());
    }
    out
}

/// The picker; returns the emoji clicked, which then counts as used lately.
pub fn picker(ui: &mut Ui, t: &Tokens, id: egui::Id) -> Option<String> {
    let mut state: State = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    let recent = recent(ui.ctx());
    let width = COLUMNS as f32 * CELL;
    let mut picked = None;
    ui.set_width(width + 12.0);
    helder::search_field(ui, t, &mut state.query, id.with("search"), &tr("emoji-search"), width + 12.0);
    ui.add_space(6.0);
    let rows = rows(&recent, &state.query);
    let total: f32 = rows.iter().map(Row::height).sum();
    let mut area = egui::ScrollArea::vertical().id_salt(id.with("list")).max_height(HEIGHT).min_scrolled_height(HEIGHT).auto_shrink([false, false]);
    if let Some(target) = state.jump.take() {
        let mut y = 0.0;
        for row in &rows {
            if let Row::Heading(_, g) = row
                && *g == target
            {
                break;
            }
            y += row.height();
        }
        area = area.vertical_scroll_offset(y);
    }
    let out = area.show_viewport(ui, |ui, viewport| {
        ui.set_height(total.max(HEIGHT));
        ui.set_width(width);
        let origin = ui.min_rect().min;
        if rows.is_empty() {
            ui.label(RichText::new(tr("emoji-none")).color(t.text_dim));
            return None;
        }
        let mut y = 0.0;
        let mut top = None;
        for (i, row) in rows.iter().enumerate() {
            let h = row.height();
            if let Row::Heading(_, g) = row
                && y <= viewport.min.y + 1.0
            {
                top = *g;
            }
            if y + h >= viewport.min.y && y <= viewport.max.y {
                let at = origin + vec2(0.0, y);
                match row {
                    Row::Heading(name, _) => {
                        let rect = Rect::from_min_size(at, vec2(width, h));
                        ui.painter().text(
                            rect.left_bottom() + vec2(4.0, -8.0),
                            egui::Align2::LEFT_BOTTOM,
                            name,
                            egui::FontId::proportional(13.0),
                            t.text_dim,
                        );
                    }
                    Row::Emoji(list) => {
                        for (c, e) in list.iter().enumerate() {
                            let rect = Rect::from_min_size(at + vec2(c as f32 * CELL, 0.0), Vec2::splat(CELL));
                            let r = ui.interact(rect, id.with(("emoji", i, c)), Sense::click());
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, *e));
                            if r.hovered() {
                                ui.painter().rect_filled(rect.shrink(1.0), 6.0, t.hover);
                            }
                            let inner = Rect::from_center_size(rect.center(), Vec2::splat(PICTURE));
                            match crate::emoji::image_of(ui.ctx(), e, PICTURE) {
                                Some(img) => img.paint_at(ui, inner),
                                None => {
                                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, *e, egui::FontId::proportional(20.0), t.text);
                                }
                            }
                            if r.clicked() {
                                picked = Some((*e).to_string());
                            }
                        }
                    }
                }
            }
            y += h;
        }
        Some(top)
    });
    if let Some(top) = out.inner {
        state.shown = top;
    }
    // The tabs: the emoji used lately, then each group.
    if state.query.trim().is_empty() {
        ui.add_space(4.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let size = vec2(width / 9.0, 30.0);
            if !recent.is_empty() {
                let lit = state.shown.is_none();
                let r = ui.add(egui::Button::image(Icon::Clock.image(18.0, if lit { t.text } else { t.text_dim })).frame(lit).min_size(size));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("emoji-recent")));
                if r.on_hover_text(tr("emoji-recent")).clicked() {
                    state.jump = Some(None);
                }
            }
            for g in Group::ALL {
                let lit = state.shown == Some(g);
                let button = match crate::emoji::image_of(ui.ctx(), g.tab(), 18.0) {
                    Some(img) => egui::Button::image(img.tint(if lit { egui::Color32::WHITE } else { egui::Color32::from_white_alpha(150) })),
                    None => egui::Button::new(g.tab()),
                };
                let r = ui.add(button.frame(lit).min_size(size));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, g.name()));
                if r.on_hover_text(g.name()).clicked() {
                    state.jump = Some(Some(g));
                }
            }
        });
    }
    if let Some(e) = &picked {
        remember(ui.ctx(), e);
        state.query.clear();
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_group_has_its_emoji_and_each_is_on_the_sheet() {
        let catalog = catalog();
        assert_eq!(catalog.iter().map(|(g, _)| *g).collect::<Vec<_>>(), Group::ALL);
        let count: usize = catalog.iter().map(|(_, l)| l.len()).sum();
        assert!(count > 1500, "the whole set, not a handful: {count}");
        for (_, list) in catalog {
            for e in list {
                assert!(crate::emoji::has_emoji(e.text), "{} has a picture", e.text);
            }
        }
        for g in Group::ALL {
            assert!(crate::emoji::has_emoji(g.tab()), "the tab of {g:?} has a picture");
        }
    }

    #[test]
    fn emoji_are_found_by_dutch_and_english_words() {
        let find = |q: &str| -> Vec<&str> {
            rows(&[], q)
                .into_iter()
                .flat_map(|r| match r {
                    Row::Emoji(l) => l,
                    Row::Heading(..) => Vec::new(),
                })
                .collect()
        };
        assert!(find("hart").contains(&"❤️") || find("hart").contains(&"❤"), "{:?}", find("hart"));
        assert!(find("heart").iter().any(|e| e.starts_with('❤')));
        assert!(find("kat").contains(&"🐱"));
        assert!(find("lach").contains(&"😂"));
        assert!(find("zzzqqq").is_empty());
    }

    #[test]
    fn the_emoji_used_lately_come_first_newest_first() {
        let ctx = egui::Context::default();
        remember(&ctx, "😀");
        remember(&ctx, "🐱");
        remember(&ctx, "😀");
        assert_eq!(recent(&ctx), ["😀", "🐱"]);
        let used = recent(&ctx);
        let rows = rows(&used, "");
        assert!(matches!(&rows[0], Row::Heading(_, None)));
        assert!(matches!(&rows[1], Row::Emoji(l) if l == &["😀", "🐱"]));
    }
}
