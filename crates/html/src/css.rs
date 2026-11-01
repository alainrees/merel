//! The little of CSS and of the old presentation attributes (`bgcolor`, `width`, `align`,
//! `<font color>`) that mail uses to lay itself out. Every value is read into a number, a
//! colour or a fixed choice; nothing from the message is passed on as text, so there is no
//! way for a value to reach anything that interprets it.

use std::collections::HashMap;

use crate::{Align, Length, Rgb, VAlign};

/// Longest `style` attribute read, with the class rules that apply to it; the rest is ignored.
const MAX_STYLE: usize = 8192;
/// Longest `<style>` text read, all blocks together.
pub const MAX_SHEET: usize = 64 * 1024;
/// Most class rules kept from it.
const MAX_RULES: usize = 1000;

/// The rules of the message's `<style>` blocks that name a single class (`.button`,
/// `div.column`), as mail tools such as MJML write them for the columns of a newsletter. Rules
/// for a narrow screen (`max-width` media queries) are left out, as Outlook on a computer does;
/// every other selector is ignored. What a rule says is read like a `style` attribute, into
/// numbers and fixed choices.
#[derive(Default)]
pub struct Sheet {
    /// Class to (declarations, declarations marked `!important`).
    rules: HashMap<String, (String, String)>,
}

impl Sheet {
    pub fn parse(css: &str) -> Self {
        let mut sheet = Sheet::default();
        let mut css: String = css.chars().take(MAX_SHEET).collect();
        // Comments out.
        while let Some(start) = css.find("/*") {
            let end = css[start + 2..].find("*/").map_or(css.len(), |e| start + 2 + e + 2);
            css.replace_range(start..end, " ");
        }
        // HTML comment markers some mail still wraps its CSS in.
        let css = css.replace("<!--", " ").replace("-->", " ");
        sheet.rules_of(&css, 0);
        sheet
    }

    fn rules_of(&mut self, mut css: &str, depth: usize) {
        while let Some(open) = css.find('{') {
            let prelude = css[..open].trim();
            let rest = &css[open + 1..];
            // The end of this block, counting nested braces.
            let mut level = 1;
            let mut close = rest.len();
            for (i, c) in rest.char_indices() {
                match c {
                    '{' => level += 1,
                    '}' => {
                        level -= 1;
                        if level == 0 {
                            close = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let body = &rest[..close];
            css = rest.get(close + 1..).unwrap_or("");
            // A prelude may still hold the end of an at-rule without a block (`@import …;`).
            let prelude = prelude.rsplit(';').next().unwrap_or("").trim().to_ascii_lowercase();
            if let Some(media) = prelude.strip_prefix("@media") {
                let narrow = media.contains("max-width") || media.contains("max-device-width");
                if !narrow && depth < 2 {
                    self.rules_of(body, depth + 1);
                }
                continue;
            }
            if prelude.starts_with('@') || body.contains('{') {
                continue;
            }
            let (mut normal, mut important) = (String::new(), String::new());
            for decl in body.split(';') {
                if !decl.contains(':') {
                    continue;
                }
                let target = if decl.to_ascii_lowercase().contains("!important") { &mut important } else { &mut normal };
                target.push_str(decl.trim());
                target.push(';');
            }
            for selector in prelude.split(',') {
                let Some(class) = single_class(selector.trim()) else { continue };
                if self.rules.len() >= MAX_RULES && !self.rules.contains_key(class) {
                    continue;
                }
                let entry = self.rules.entry(class.to_string()).or_default();
                entry.0.push_str(&normal);
                entry.1.push_str(&important);
            }
        }
    }

    /// What the rules say for an element of these classes (its `class` attribute), to be read
    /// before its own `style`, and after it for what is marked `!important`.
    pub fn for_classes(&self, classes: &str) -> (String, String) {
        let (mut normal, mut important) = (String::new(), String::new());
        if self.rules.is_empty() {
            return (normal, important);
        }
        for class in classes.split_ascii_whitespace().take(16) {
            if let Some((n, i)) = self.rules.get(&class.to_ascii_lowercase()) {
                normal.push_str(n);
                important.push_str(i);
            }
        }
        (normal, important)
    }
}

/// The class a selector names when it names nothing else: `.x` or `div.x`.
fn single_class(selector: &str) -> Option<&str> {
    let (tag, class) = selector.split_once('.')?;
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ((tag.is_empty() || word(tag)) && word(class)).then_some(class)
}

/// The declarations of one `style` attribute, later ones first so the last one wins.
pub struct Declarations(Vec<(String, String)>);

impl Declarations {
    pub fn parse(style: &str) -> Self {
        let mut out: Vec<(String, String)> = Vec::new();
        let style: String = style.chars().take(MAX_STYLE).collect();
        for decl in style.split(';') {
            let Some((name, value)) = decl.split_once(':') else { continue };
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            let value = value.strip_suffix("!important").unwrap_or(value).trim().to_ascii_lowercase();
            if !name.is_empty() && !value.is_empty() {
                out.push((name, value));
            }
        }
        out.reverse();
        Declarations(out)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// A colour as mail writes it: `#rgb`, `#rrggbb`, `rgb(…)` or a common name.
pub fn color(value: &str) -> Option<Rgb> {
    let v = value.trim().to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#').or_else(|| (v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit())).then_some(v.as_str())) {
        let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
        let pair = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        return match hex.len() {
            3 => Some(Rgb(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
            6 => Some(Rgb(pair(0)?, pair(2)?, pair(4)?)),
            _ => None,
        };
    }
    if let Some(inner) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let parts: Vec<&str> = inner.trim_end_matches(')').split([',', ' ', '/']).filter(|p| !p.is_empty()).collect();
        if let Some(alpha) = parts.get(3).and_then(|a| a.trim_end_matches('%').parse::<f32>().ok())
            && alpha == 0.0
        {
            return None;
        }
        let channel = |p: &str| -> Option<u8> {
            match p.strip_suffix('%') {
                Some(pct) => pct.parse::<f32>().ok().map(|f| (f.clamp(0.0, 100.0) * 2.55).round() as u8),
                None => p.parse::<f32>().ok().map(|f| f.clamp(0.0, 255.0).round() as u8),
            }
        };
        return Some(Rgb(channel(parts.first()?)?, channel(parts.get(1)?)?, channel(parts.get(2)?)?));
    }
    let named = match v.as_str() {
        "white" => (255, 255, 255),
        "black" => (0, 0, 0),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "orange" => (255, 165, 0),
        "gray" | "grey" => (128, 128, 128),
        "silver" => (192, 192, 192),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "whitesmoke" => (245, 245, 245),
        "navy" => (0, 0, 128),
        "maroon" => (128, 0, 0),
        "purple" => (128, 0, 128),
        "teal" => (0, 128, 128),
        "olive" => (128, 128, 0),
        "lime" => (0, 255, 0),
        "aqua" | "cyan" => (0, 255, 255),
        "fuchsia" | "magenta" => (255, 0, 255),
        _ => return None,
    };
    Some(Rgb(named.0, named.1, named.2))
}

/// The first colour among the words of a `background` shorthand.
pub fn background(value: &str) -> Option<Rgb> {
    if let Some(c) = color(value) {
        return Some(c);
    }
    let mut rest = value;
    while !rest.is_empty() {
        let rest_trim = rest.trim_start();
        // `rgb(1, 2, 3)` holds spaces; take it whole.
        let end = if rest_trim.starts_with("rgb") {
            rest_trim.find(')').map_or(rest_trim.len(), |i| i + 1)
        } else {
            rest_trim.find(' ').unwrap_or(rest_trim.len())
        };
        let (word, tail) = rest_trim.split_at(end);
        if let Some(c) = color(word) {
            return Some(c);
        }
        rest = tail;
    }
    None
}

/// A length in pixels: `12`, `12px`, `9pt` or `1.5em` (of 16 px).
pub fn px(value: &str) -> Option<f32> {
    let v = value.trim().to_ascii_lowercase();
    let (number, factor) = if let Some(n) = v.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = v.strip_suffix("pt") {
        (n, 4.0 / 3.0)
    } else if let Some(n) = v.strip_suffix("rem").or_else(|| v.strip_suffix("em")) {
        (n, 16.0)
    } else {
        (v.as_str(), 1.0)
    };
    let n: f32 = number.trim().parse().ok()?;
    (n.is_finite() && n >= 0.0).then_some(n * factor)
}

/// A width: pixels or a percentage of the room there is.
pub fn length(value: &str) -> Option<Length> {
    let v = value.trim();
    if let Some(p) = v.strip_suffix('%') {
        let p: f32 = p.trim().parse().ok()?;
        return (p.is_finite() && p > 0.0).then(|| Length::Percent(p.min(100.0).round() as u8));
    }
    let px = px(v)?;
    (px > 0.0).then(|| Length::Px(px.min(f32::from(u16::MAX)).round() as u16))
}

/// A pixel count that fits the layout tree.
pub fn short(value: f32) -> u16 {
    value.clamp(0.0, f32::from(u16::MAX)).round() as u16
}

/// `padding`, `margin` and the like: one to four lengths, as top, right, bottom, left.
pub fn sides(value: &str) -> Option<[u16; 4]> {
    let v: Vec<u16> = value.split_whitespace().map(|w| px(w).map(short)).collect::<Option<_>>()?;
    match v.as_slice() {
        [a] => Some([*a; 4]),
        [a, b] => Some([*a, *b, *a, *b]),
        [a, b, c] => Some([*a, *b, *c, *b]),
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}

pub fn align(value: &str) -> Option<Align> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" | "start" | "justify" => Some(Align::Left),
        "center" | "middle" | "-webkit-center" => Some(Align::Center),
        "right" | "end" => Some(Align::Right),
        _ => None,
    }
}

/// The rounding of the corners in `border-radius`: top left, top right, bottom right, bottom
/// left. Only the horizontal radii (before a `/`) count.
pub fn radius(value: &str) -> Option<[u16; 4]> {
    let corners = sides(value.split('/').next()?)?;
    corners.iter().any(|&r| r > 0).then_some(corners)
}

pub fn valign(value: &str) -> Option<VAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "top" => Some(VAlign::Top),
        "middle" | "center" => Some(VAlign::Middle),
        "bottom" => Some(VAlign::Bottom),
        _ => None,
    }
}

/// The size `<font size>` stands for, in pixels.
pub fn font_size(value: &str) -> Option<f32> {
    let v = value.trim();
    let n: i32 = v.parse().ok()?;
    let n = if v.starts_with(['+', '-']) { 3 + n } else { n };
    Some([10.0, 13.0, 16.0, 18.0, 24.0, 32.0, 48.0][n.clamp(1, 7) as usize - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_read_into_numbers_and_colours() {
        assert_eq!(color("#F60"), Some(Rgb(255, 102, 0)));
        assert_eq!(color("#ec7a31"), Some(Rgb(0xec, 0x7a, 0x31)));
        assert_eq!(color("ec7a31"), Some(Rgb(0xec, 0x7a, 0x31)), "bgcolor without the #");
        assert_eq!(color("rgb(10, 20, 30)"), Some(Rgb(10, 20, 30)));
        assert_eq!(color("rgba(0,0,0,0)"), None, "transparent");
        assert_eq!(color("White"), Some(Rgb(255, 255, 255)));
        assert_eq!(color("url(javascript:x)"), None);
        assert_eq!(background("url(https://t.example/x.png) no-repeat #eaeef2"), Some(Rgb(0xea, 0xee, 0xf2)));
        assert_eq!(length("600"), Some(Length::Px(600)));
        assert_eq!(length("60px"), Some(Length::Px(60)));
        assert_eq!(length("100%"), Some(Length::Percent(100)));
        assert_eq!(length("auto"), None);
        assert_eq!(sides("10px 20px"), Some([10, 20, 10, 20]));
        assert_eq!(px("12pt"), Some(16.0));
        assert_eq!(font_size("+1"), Some(18.0));
        let d = Declarations::parse("COLOR: #fff; width:100px !important; color:#000;garbage");
        assert_eq!(d.get("color"), Some("#000"), "the last one wins");
        assert_eq!(d.get("width"), Some("100px"));
        assert_eq!(radius("24px"), Some([24; 4]));
        assert_eq!(radius("8px 8px 0 0"), Some([8, 8, 0, 0]));
        assert_eq!(radius("0"), None);
    }

    #[test]
    fn class_rules_for_a_wide_screen_are_read_and_the_rest_ignored() {
        let sheet = Sheet::parse(
            "<!-- #outlook a { padding:0; } body{margin:0} /* .x{width:1px} */
            @media only screen and (min-width:480px) { .mj-column-per-20 { width:20% !important; max-width: 20%; } .moz-text-html .mj-column-per-20 { width:99% } }
            @media only screen and (max-width:480px) { .mj-column-per-20 { width:100% !important; } }
            @import url(https://t.example/a.css); div.Card, .button { border-radius: 8px } @font-face { font-family: X; src: url(x) } -->",
        );
        let (normal, important) = sheet.for_classes("mj-column-per-20 mj-outlook-group-fix");
        assert_eq!((normal.as_str(), important.as_str()), ("max-width: 20%;", "width:20% !important;"));
        assert_eq!(sheet.for_classes("card").0, "border-radius: 8px;");
        assert_eq!(sheet.for_classes("button").0, "border-radius: 8px;");
        assert_eq!(sheet.for_classes("x"), (String::new(), String::new()));
        let d = Declarations::parse(&format!("{normal};width:100%;{important}"));
        assert_eq!(d.get("width"), Some("20%"), "!important in a class beats the element's own style");
    }
}
