//! The app's fonts. egui's own text font (Ubuntu Light) is thin and reads as grey, and its
//! symbol fonts miss arrows and shapes the ribbon uses, so the app brings two of its own:
//! Inter for text (OFL) and DejaVu Sans (Bitstream Vera licence) for the symbols. Both are
//! embedded in the binary; their licences sit next to them under `assets/fonts`.
//!
//! The message editor offers three typefaces in bold and italic too, so the app also carries
//! Inter's bold and italic cuts, DejaVu Serif and DejaVu Sans Mono; each cut is a font family
//! of its own (see [`family`]), with the symbol fonts behind it.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};
use merel_model::rich::Face;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.otf");
const DEJAVU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
/// The cuts of the editor's typefaces: (name, bytes).
const CUTS: [(&str, &[u8]); 7] = [
    ("Inter-Bold", include_bytes!("../assets/fonts/Inter-Bold.otf")),
    ("Inter-Italic", include_bytes!("../assets/fonts/Inter-Italic.otf")),
    ("Inter-BoldItalic", include_bytes!("../assets/fonts/Inter-BoldItalic.otf")),
    ("DejaVuSerif", include_bytes!("../assets/fonts/DejaVuSerif.ttf")),
    ("DejaVuSerif-Bold", include_bytes!("../assets/fonts/DejaVuSerif-Bold.ttf")),
    ("DejaVuSansMono", include_bytes!("../assets/fonts/DejaVuSansMono.ttf")),
    ("DejaVuSansMono-Bold", include_bytes!("../assets/fonts/DejaVuSansMono-Bold.ttf")),
];

/// The font a cut of `face` is drawn with, and whether egui has to slant it because the app
/// has no italic of its own for that face.
fn cut(face: Face, bold: bool, italic: bool) -> (&'static str, bool) {
    match (face, bold, italic) {
        (Face::Sans, false, false) => ("Inter", false),
        (Face::Sans, true, false) => ("Inter-Bold", false),
        (Face::Sans, false, true) => ("Inter-Italic", false),
        (Face::Sans, true, true) => ("Inter-BoldItalic", false),
        (Face::Serif, false, i) => ("DejaVuSerif", i),
        (Face::Serif, true, i) => ("DejaVuSerif-Bold", i),
        (Face::Mono, false, i) => ("DejaVuSansMono", i),
        (Face::Mono, true, i) => ("DejaVuSansMono-Bold", i),
    }
}

/// The family that draws `face` in bold and/or italic, and whether egui must slant it.
pub fn family(face: Face, bold: bool, italic: bool) -> (FontFamily, bool) {
    let (name, slant) = cut(face, bold, italic);
    (FontFamily::Name(format!("rich-{name}").into()), slant)
}

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
    for (name, bytes) in CUTS {
        defs.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    // Each cut first, then the symbol fonts proportional text falls back on.
    let fallbacks: Vec<String> = defs.families.get(&FontFamily::Proportional).map(|f| f[1..].to_vec()).unwrap_or_default();
    for face in Face::ALL {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let (name, _) = cut(face, bold, italic);
            let (family, _) = family(face, bold, italic);
            defs.families.insert(family, std::iter::once(name.to_owned()).chain(fallbacks.iter().cloned()).collect());
        }
    }
    defs
}
