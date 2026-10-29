//! The little of CSS and of the old presentation attributes (`bgcolor`, `width`, `align`,
//! `<font color>`) that mail uses to lay itself out. Every value is read into a number, a
//! colour or a fixed choice; nothing from the message is passed on as text, so there is no
//! way for a value to reach anything that interprets it.

use crate::{Align, Length, Rgb, VAlign};

/// Longest `style` attribute read; the rest is ignored.
const MAX_STYLE: usize = 4096;

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
    }
}
