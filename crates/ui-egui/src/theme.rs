//! Colours, in the ArtCraft family style: neutral greys with one app colour.
//!
//! Every colour pair the window draws meets WCAG 2.2 level AA: text at 4.5:1 or better on
//! each surface it can sit on (disabled text and placeholders included, although AA exempts
//! them), icons and control borders at 3:1 or better. [`contrast`] measures it and a test
//! holds both themes to it.

use egui::{Color32, Stroke};

/// Merel's app colour (each ArtCraft app has its own; WordCraft's is #3B5BDB).
pub const APP_COLOR: Color32 = Color32::from_rgb(0x1F, 0x6F, 0xC9);
pub const APP_INK: Color32 = Color32::from_rgb(0x17, 0x57, 0xA3);

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    pub title_bar: Color32,
    pub ribbon: Color32,
    pub tab_strip: Color32,
    pub rail: Color32,
    pub panel: Color32,
    pub list: Color32,
    pub reading: Color32,
    pub border: Color32,
    pub text: Color32,
    /// Secondary text: captions, previews, the status bar. Still 4.5:1 on every surface.
    pub text_dim: Color32,
    /// Text and glyphs of a control that cannot be used right now: told apart from enabled
    /// text by its grey, but still readable at 4.5:1.
    pub text_disabled: Color32,
    /// The border of a checkbox, text field or button: 3:1 or better on every surface, so the
    /// control's edge can be seen. Panel seams keep the lighter `border`.
    pub control_border: Color32,
    pub hover: Color32,
    pub checked: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub status_bar: Color32,
    pub flag: Color32,
    /// The icon and edge of a notice that says an action worked; red stays for what failed.
    pub success: Color32,
    /// The icon and edge of a notice that asks for care.
    pub warning: Color32,
    /// The ground of a window in the Helder style, between its cards.
    pub window: Color32,
    /// A card: a rounded panel that holds a form, a list or a preview.
    pub card: Color32,
    /// The seam around a card; a control's edge stays `control_border`.
    pub card_border: Color32,
}

impl Tokens {
    pub fn light() -> Self {
        Tokens {
            dark: false,
            title_bar: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            ribbon: Color32::from_rgb(0xF9, 0xF9, 0xF9),
            tab_strip: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            rail: Color32::from_rgb(0xEC, 0xEC, 0xEC),
            panel: Color32::from_rgb(0xFA, 0xFA, 0xFA),
            list: Color32::WHITE,
            reading: Color32::WHITE,
            border: Color32::from_rgb(0xE1, 0xE1, 0xE1),
            text: Color32::from_rgb(0x24, 0x24, 0x24),
            text_dim: Color32::from_rgb(0x61, 0x61, 0x61),
            text_disabled: Color32::from_rgb(0x5E, 0x5E, 0x5E),
            control_border: Color32::from_rgb(0x7A, 0x7A, 0x7A),
            hover: Color32::from_rgb(0xE6, 0xE6, 0xE6),
            checked: Color32::from_rgb(0xDC, 0xE8, 0xF7),
            accent: APP_COLOR,
            accent_text: APP_INK,
            on_accent: Color32::WHITE,
            status_bar: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            flag: Color32::from_rgb(0xC4, 0x31, 0x4B),
            success: Color32::from_rgb(0x10, 0x7C, 0x10),
            warning: Color32::from_rgb(0xB2, 0x5E, 0x00),
            window: Color32::from_rgb(0xF3, 0xF4, 0xF6),
            card: Color32::WHITE,
            card_border: Color32::from_rgb(0xE3, 0xE5, 0xE8),
        }
    }

    pub fn dark() -> Self {
        Tokens {
            dark: true,
            title_bar: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            ribbon: Color32::from_rgb(0x29, 0x29, 0x29),
            tab_strip: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            rail: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            panel: Color32::from_rgb(0x26, 0x26, 0x26),
            list: Color32::from_rgb(0x20, 0x20, 0x20),
            reading: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            border: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            text: Color32::from_rgb(0xEE, 0xEE, 0xEE),
            text_dim: Color32::from_rgb(0xC8, 0xC8, 0xC8),
            text_disabled: Color32::from_rgb(0xAC, 0xAC, 0xAC),
            control_border: Color32::from_rgb(0x8A, 0x8A, 0x8A),
            hover: Color32::from_rgb(0x3D, 0x3D, 0x3D),
            checked: Color32::from_rgb(0x1E, 0x35, 0x52),
            accent: Color32::from_rgb(0x6F, 0xA8, 0xEE),
            accent_text: Color32::from_rgb(0x95, 0xC0, 0xF4),
            // Dark text on the light dark-theme accent; white would be under 4.5:1.
            on_accent: Color32::from_rgb(0x0B, 0x1A, 0x2E),
            status_bar: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            flag: Color32::from_rgb(0xF0, 0x7A, 0x8C),
            success: Color32::from_rgb(0x6C, 0xCB, 0x5F),
            warning: Color32::from_rgb(0xF2, 0xB1, 0x4B),
            window: Color32::from_rgb(0x1B, 0x1C, 0x1E),
            card: Color32::from_rgb(0x26, 0x28, 0x2B),
            card_border: Color32::from_rgb(0x3A, 0x3D, 0x42),
        }
    }

    pub fn for_dark(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    /// The surfaces text and icons are drawn on.
    pub fn surfaces(&self) -> [Color32; 12] {
        [
            self.title_bar,
            self.ribbon,
            self.tab_strip,
            self.rail,
            self.panel,
            self.list,
            self.reading,
            self.hover,
            self.checked,
            self.status_bar,
            self.window,
            self.card,
        ]
    }

    /// Puts the tokens into egui's own widgets, so text egui colours itself (labels, buttons,
    /// checkboxes, the placeholder of a text field, a widget that is disabled) is as readable as
    /// the text the app colours: every text colour is `text` or `text_dim`, a disabled widget
    /// keeps [`DISABLED_ALPHA`] of its colour, and controls get a visible border.
    pub fn style(&self, visuals: &mut egui::Visuals) {
        for w in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            w.fg_stroke.color = self.text;
        }
        visuals.weak_text_color = Some(self.text_dim);
        // What shows between the panels: the Helder window's grey ground.
        visuals.panel_fill = self.window;
        visuals.disabled_alpha = DISABLED_ALPHA;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.control_border);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.text_dim);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, self.text);
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, self.control_border);
    }
}

/// How much of its colour a disabled egui widget keeps; chosen so greyed text still reads at
/// 4.5:1 on every surface while plainly differing from enabled text.
pub const DISABLED_ALPHA: f32 = 0.72;

/// Gives both of egui's themes the app's readable colours. Once, at start; the choice between
/// light and dark is made separately with `set_theme`.
pub fn install(ctx: &egui::Context) {
    for (theme, tokens) in [(egui::Theme::Light, Tokens::light()), (egui::Theme::Dark, Tokens::dark())] {
        ctx.style_mut_of(theme, |style| tokens.style(&mut style.visuals));
    }
}

/// The WCAG 2.2 contrast ratio between two opaque colours, from 1 (the same) to 21 (black on
/// white). Text needs 4.5 for level AA, large text, icons and control borders 3.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `fg` drawn at `alpha` over `bg`, as the screen shows it.
pub fn over(fg: Color32, bg: Color32, alpha: f32) -> Color32 {
    let mix = |f: u8, b: u8| (f as f32 * alpha + b as f32 * (1.0 - alpha)).round() as u8;
    Color32::from_rgb(mix(fg.r(), bg.r()), mix(fg.g(), bg.g()), mix(fg.b(), bg.b()))
}

/// Relative luminance as WCAG defines it, for sRGB.
fn luminance(c: Color32) -> f32 {
    let channel = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Color32) -> String {
        format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
    }

    #[test]
    fn contrast_follows_the_wcag_formula() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.01);
        // #767676 on white is the classic 4.54:1 pair.
        assert!((contrast(Color32::from_gray(0x76), Color32::WHITE) - 4.54).abs() < 0.02);
    }

    #[test]
    fn both_themes_meet_wcag_2_2_level_aa() {
        let mut failures = Vec::new();
        for t in [Tokens::light(), Tokens::dark()] {
            let theme = if t.dark { "dark" } else { "light" };
            let mut need = |what: &str, fg: Color32, bg: Color32, least: f32| {
                let ratio = contrast(fg, bg);
                if ratio < least {
                    failures.push(format!("{theme}: {what} {} on {} is {ratio:.2}:1, needs {least}:1", hex(fg), hex(bg)));
                }
            };
            for bg in t.surfaces() {
                for (what, fg) in [("text", t.text), ("text_dim", t.text_dim), ("text_disabled", t.text_disabled), ("accent_text", t.accent_text)] {
                    need(what, fg, bg, 4.5);
                }
                need("disabled widget text", over(t.text, bg, DISABLED_ALPHA), bg, 4.5);
                for (what, fg) in [
                    ("accent icon", t.accent),
                    ("flag icon", t.flag),
                    ("success icon", t.success),
                    ("warning icon", t.warning),
                    ("control border", t.control_border),
                ] {
                    need(what, fg, bg, 3.0);
                }
            }
            need("on_accent", t.on_accent, t.accent, 4.5);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
