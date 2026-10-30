//! Emoji in colour, as Signal shows them. egui draws text from outline fonts only, in one
//! colour, so the computer's own colour emoji (Apple Color Emoji, Segoe UI Emoji) cannot be
//! used. Instead the chats draw each emoji as a picture from Noto Color Emoji: one sheet with
//! all of them, put on the graphics card once; the fonts' one-colour emoji stay for the rest.

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{Color32, Rect, RichText, Ui, Vec2, pos2, vec2};

/// Every emoji in a grid of `CELL` pixels, each picture `SIDE` pixels with a pixel around it.
const SHEET: &[u8] = include_bytes!("../assets/emoji/noto-emoji-64.png");
/// Where each emoji is on the sheet: its code points in hex, joined by `-`, and its column and row.
const INDEX: &str = include_str!("../assets/emoji/noto-emoji-64.txt");
const CELL: f32 = 66.0;
const SIDE: f32 = 64.0;
const GRID: f32 = 62.0;
/// Most code points in one emoji: a family with skin tones has about ten.
const LONGEST: usize = 12;

fn index() -> &'static HashMap<String, (u8, u8)> {
    static INDEX_MAP: OnceLock<HashMap<String, (u8, u8)>> = OnceLock::new();
    INDEX_MAP.get_or_init(|| {
        INDEX
            .lines()
            .filter_map(|line| {
                let mut parts = line.split(' ');
                let code = parts.next()?;
                let x = parts.next()?.parse().ok()?;
                let y = parts.next()?.parse().ok()?;
                let key: String = code.split('-').filter_map(|h| u32::from_str_radix(h, 16).ok()).filter_map(char::from_u32).collect();
                Some((key, (x, y)))
            })
            .collect()
    })
}

/// A piece of text: plain, or one emoji with its place on the sheet.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    Text(&'a str),
    Emoji(&'a str, (u8, u8)),
}

/// The emoji at the start of `text`, longest first, with its length in bytes.
fn emoji_at(text: &str) -> Option<(usize, (u8, u8))> {
    let first = text.chars().next()?;
    if first.is_ascii() && !matches!(first, '#' | '*' | '0'..='9') {
        return None;
    }
    let ends: Vec<usize> = text.char_indices().skip(1).map(|(i, _)| i).chain([text.len()]).take(LONGEST).collect();
    ends.iter().rev().find_map(|&end| index().get(&text[..end]).map(|&at| (end, at)))
}

/// `text` cut into plain text and emoji.
pub(crate) fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut plain = 0;
    let mut i = 0;
    while i < text.len() {
        match emoji_at(&text[i..]) {
            // A digit or # is only an emoji as a keycap, which is longer than the digit.
            Some((len, at)) if !(len == 1 && text[i..].starts_with(|c: char| c.is_ascii())) => {
                if plain < i {
                    out.push(Piece::Text(&text[plain..i]));
                }
                // A variation selector after the emoji belongs to it.
                let mut end = i + len;
                if text[end..].starts_with('\u{FE0F}') {
                    end += '\u{FE0F}'.len_utf8();
                }
                out.push(Piece::Emoji(&text[i..end], at));
                i = end;
                plain = i;
            }
            _ => i += text[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    if plain < text.len() {
        out.push(Piece::Text(&text[plain..]));
    }
    out
}

/// Whether `text` has an emoji this sheet has.
#[cfg(test)]
pub(crate) fn has_emoji(text: &str) -> bool {
    pieces(text).iter().any(|p| matches!(p, Piece::Emoji(..)))
}

/// The sheet on the graphics card, loaded the first time it is needed.
fn sheet(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("merel-emoji-sheet");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(t);
    }
    let decoded = image::load_from_memory_with_format(SHEET, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let picture = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let texture = ctx.load_texture("merel-emoji", picture, options);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

/// The emoji at `at` on the sheet, `side` points wide.
pub(crate) fn image(ctx: &egui::Context, at: (u8, u8), side: f32) -> Option<egui::Image<'static>> {
    let texture = sheet(ctx)?;
    let full = CELL * GRID;
    let min = pos2(f32::from(at.0) * CELL + 1.0, f32::from(at.1) * CELL + 1.0);
    let uv = Rect::from_min_size(pos2(min.x / full, min.y / full), vec2(SIDE / full, SIDE / full));
    Some(egui::Image::new((texture.id(), Vec2::splat(side))).uv(uv))
}

/// The picture for the one emoji `text` is, if it is one.
pub(crate) fn image_of(ctx: &egui::Context, text: &str, side: f32) -> Option<egui::Image<'static>> {
    match pieces(text).as_slice() {
        [Piece::Emoji(_, at)] => image(ctx, *at, side),
        _ => None,
    }
}

/// The web links in `text`, as Signal finds them: `http://`, `https://` or `www.` up to the
/// next space, without the full stop or bracket that ends a sentence. Each with the address
/// it opens, always over http or https: nothing else is a link.
pub(crate) fn links(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let lower = rest.get(..8).unwrap_or(rest).to_ascii_lowercase();
        let starts = lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("www.");
        let at_word = text[..i].chars().next_back().is_none_or(|c| c.is_whitespace() || "([<\"'".contains(c));
        if !(starts && at_word) {
            i += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let mut end = i + rest.find(char::is_whitespace).unwrap_or(rest.len());
        // A sentence's full stop or a closing bracket is not part of the address, unless the
        // address opened that bracket itself (as Wikipedia's do).
        while let Some(c) = text[i..end].chars().next_back() {
            let opened = text[i..end].matches('(').count() >= text[i..end].matches(')').count();
            if ".,;:!?'\"»>]".contains(c) || (c == ')' && !opened) {
                end -= c.len_utf8();
            } else {
                break;
            }
        }
        let found = &text[i..end];
        let scheme = found.find("://").map_or(0, |n| n + 3);
        if found.len() > scheme + 3 && found[scheme..].contains('.') {
            let url = if scheme == 0 { format!("https://{found}") } else { found.to_string() };
            out.push((i..end, url));
        }
        i = end.max(i + 1);
        while !text.is_char_boundary(i) {
            i += 1;
        }
    }
    out
}

/// One link: underlined in the text's own colour, so it is as easy to read as the rest, with
/// the address it opens on hover; a click opens it in the browser.
fn link(ui: &mut Ui, shown: &str, url: &str, size: f32, color: Color32) -> egui::Response {
    let r = ui.add(egui::Label::new(RichText::new(shown).size(size).color(color).underline()).sense(egui::Sense::click()).wrap());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(url);
    if r.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
    r
}

/// Text that wraps, with its emoji in colour (a little larger than the letters, as in Signal)
/// and its web links to click. Text without either is a plain label. Gives the clicks on all
/// of it together, for a right-click menu.
pub(crate) fn label(ui: &mut Ui, text: &str, size: f32, color: Color32) -> egui::Response {
    let pieces = pieces(text);
    let found = links(text);
    if found.is_empty() && !pieces.iter().any(|p| matches!(p, Piece::Emoji(..))) {
        return ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).wrap());
    }
    // A message of only emoji shows them big.
    let only = pieces.iter().all(|p| matches!(p, Piece::Emoji(..)) || matches!(p, Piece::Text(t) if t.trim().is_empty()));
    let side = if only && pieces.len() <= 3 { size * 2.4 } else { size * 1.3 };
    let wrapped = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        let mut all: Option<egui::Response> = None;
        let mut add = |r: egui::Response| {
            all = Some(match all.take() {
                Some(a) => a.union(r),
                None => r,
            })
        };
        // Where each piece starts in `text`, to find the links in it.
        let mut at = 0;
        for piece in pieces {
            match piece {
                Piece::Text(t) => {
                    let start = at;
                    at += t.len();
                    let mut from = start;
                    for (range, url) in found.iter().filter(|(r, _)| r.start >= start && r.end <= at) {
                        if from < range.start {
                            add(ui.add(egui::Label::new(RichText::new(&text[from..range.start]).size(size).color(color)).wrap()));
                        }
                        add(link(ui, &text[range.clone()], url, size, color));
                        from = range.end;
                    }
                    if from < at {
                        add(ui.add(egui::Label::new(RichText::new(&text[from..at]).size(size).color(color)).wrap()));
                    }
                }
                Piece::Emoji(e, at_sheet) => {
                    at += e.len();
                    match image(ui.ctx(), at_sheet, side) {
                        Some(img) => {
                            let r = ui.add(img);
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, e));
                            add(r);
                        }
                        None => add(ui.label(RichText::new(e).size(size).color(color))),
                    }
                }
            }
        }
        all
    });
    wrapped.inner.unwrap_or(wrapped.response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_links_are_found_without_the_full_stop_and_only_over_the_web() {
        let found = |t: &str| links(t).into_iter().map(|(r, url)| (t[r].to_string(), url)).collect::<Vec<_>>();
        assert_eq!(
            found("Kijk op https://signal.org/nl/. Of www.merel.app!"),
            [("https://signal.org/nl/".into(), "https://signal.org/nl/".into()), ("www.merel.app".into(), "https://www.merel.app".into())]
        );
        assert_eq!(found("(zie https://nl.wikipedia.org/wiki/Merel_(vogel))")[0].0, "https://nl.wikipedia.org/wiki/Merel_(vogel)");
        assert_eq!(found("HTTP://Voorbeeld.nl/x?a=1&b=2")[0].1, "HTTP://Voorbeeld.nl/x?a=1&b=2");
        for none in ["javascript:alert(1)", "file:///etc/passwd", "geen link", "https://", "mijnwww.nl", "www.", "ftp://x.nl"] {
            assert!(links(none).is_empty(), "{none}");
        }
        assert_eq!(found("👍 https://x.nl 👍")[0].0, "https://x.nl");
    }

    #[test]
    fn emoji_are_found_with_skin_tones_and_joined_ones_whole() {
        let p = pieces("Top 👍🏽 en ❤️! 👨‍👩‍👧 #1");
        let emoji: Vec<&str> = p.iter().filter_map(|p| if let Piece::Emoji(e, _) = p { Some(*e) } else { None }).collect();
        assert_eq!(emoji, ["👍🏽", "❤️", "👨‍👩‍👧"]);
        assert_eq!(p.first(), Some(&Piece::Text("Top ")));
        assert_eq!(p.last(), Some(&Piece::Text(" #1")));
        assert!(!has_emoji("Gewoon tekst, 100% zonder 2 emoji."));
        assert!(has_emoji("1️⃣"));
    }

    #[test]
    fn every_emoji_of_the_pickers_is_on_the_sheet() {
        for e in crate::chat::all_emoji() {
            assert!(matches!(pieces(e).as_slice(), [Piece::Emoji(..)]), "{e} is not on the sheet");
        }
    }
}
