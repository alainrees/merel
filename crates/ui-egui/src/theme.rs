//! Colours, in the ArtCraft family style: neutral greys with one app colour.

use egui::Color32;

/// Mailcraft's app colour (each ArtCraft app has its own; WordCraft's is #3B5BDB).
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
    pub text_dim: Color32,
    pub hover: Color32,
    pub checked: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub status_bar: Color32,
    pub flag: Color32,
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
            hover: Color32::from_rgb(0xE6, 0xE6, 0xE6),
            checked: Color32::from_rgb(0xDC, 0xE8, 0xF7),
            accent: APP_COLOR,
            accent_text: APP_INK,
            on_accent: Color32::WHITE,
            status_bar: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            flag: Color32::from_rgb(0xC4, 0x31, 0x4B),
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
            hover: Color32::from_rgb(0x3D, 0x3D, 0x3D),
            checked: Color32::from_rgb(0x1E, 0x35, 0x52),
            accent: Color32::from_rgb(0x6F, 0xA8, 0xEE),
            accent_text: Color32::from_rgb(0x95, 0xC0, 0xF4),
            // Dark text on the light dark-theme accent; white would be under 4.5:1.
            on_accent: Color32::from_rgb(0x0B, 0x1A, 0x2E),
            status_bar: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            flag: Color32::from_rgb(0xF0, 0x7A, 0x8C),
        }
    }

    pub fn for_dark(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }
}
