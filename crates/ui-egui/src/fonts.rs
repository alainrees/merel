//! The app's fonts. egui's own text font (Ubuntu Light) is thin and reads as grey, and its
//! symbol fonts miss arrows and shapes the ribbon uses, so the app brings two of its own:
//! Inter for text (OFL) and DejaVu Sans (Bitstream Vera licence) for the symbols. Both are
//! embedded in the binary; their licences sit next to them under `assets/fonts`.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.otf");
const DEJAVU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");

/// egui's defaults with Inter first for proportional text and DejaVu Sans right after it, so
/// a symbol Inter lacks comes from DejaVu before egui's emoji fonts get a turn. Monospace
/// keeps egui's Hack with DejaVu Sans as its fallback.
pub fn definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    defs.font_data.insert("Inter".to_owned(), Arc::new(FontData::from_static(INTER)));
    defs.font_data.insert("DejaVuSans".to_owned(), Arc::new(FontData::from_static(DEJAVU_SANS)));
    let proportional = defs.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "Inter".to_owned());
    proportional.insert(1, "DejaVuSans".to_owned());
    defs.families.entry(FontFamily::Monospace).or_default().insert(1, "DejaVuSans".to_owned());
    defs
}
