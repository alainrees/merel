//! The strip over a message with what Merel recognised in its text: amounts, an IBAN, a
//! track-and-trace code, order and invoice numbers (`merel_extract::recognize`, fixed rules,
//! on this computer). A click on an order or invoice number searches all mail for it, a code
//! with a known carrier opens that carrier's page (only on the click), the rest is copied.

use std::sync::Arc;

use crate::i18n::{tr, tr_args};
use egui::{RichText, Ui};
use merel_extract::recognize::{Found, What, format_amount, recognize};

use crate::Tokens;

/// The most chips the strip shows.
const MAX_CHIPS: usize = 8;

/// What a click on a chip asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Search all mail for these words.
    Search(String),
    Copy(String),
    Open(String),
}

pub fn strip_label() -> String {
    tr("recognized-strip")
}

/// The findings of a message's text, worked out once per message.
pub fn found_in(ui: &Ui, message_id: &str, text: &str) -> Arc<Vec<Found>> {
    let id = egui::Id::new(("recognized", message_id));
    if let Some(found) = ui.data(|d| d.get_temp::<Arc<Vec<Found>>>(id)) {
        return found;
    }
    let found = Arc::new(recognize(text));
    ui.data_mut(|d| d.insert_temp(id, Arc::clone(&found)));
    found
}

/// An IBAN in groups of four, as people write it.
fn grouped(iban: &str) -> String {
    iban.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" ")
}

/// The label, hover text and click of one finding.
pub fn chip(f: &Found) -> (String, String, Pick) {
    match f.what {
        What::Amount { cents, currency } => {
            let amount = format_amount(cents, currency);
            (amount.clone(), tr("recognized-copy-hint"), Pick::Copy(amount))
        }
        What::Iban => (format!("IBAN {}", grouped(&f.value)), tr("recognized-copy-hint"), Pick::Copy(f.value.clone())),
        What::Tracking { carrier: Some(c) } => {
            let link = c.link(&f.value);
            (format!("{} {}", c.name(), f.value), tr_args("recognized-track-hint", &[("link", link.clone().into())]), Pick::Open(link))
        }
        What::Tracking { carrier: None } => {
            (tr_args("recognized-tracking", &[("code", f.value.clone().into())]), tr("recognized-copy-hint"), Pick::Copy(f.value.clone()))
        }
        What::Order => {
            (tr_args("recognized-order", &[("code", f.value.clone().into())]), tr("recognized-search-hint"), Pick::Search(format!("\"{}\"", f.value)))
        }
        What::Invoice => (
            tr_args("recognized-invoice", &[("code", f.value.clone().into())]),
            tr("recognized-search-hint"),
            Pick::Search(format!("\"{}\"", f.value)),
        ),
    }
}

/// The strip; what the person clicked, if anything. Nothing when nothing was recognised.
pub fn strip(ui: &mut Ui, t: &Tokens, found: &[Found]) -> Option<Pick> {
    if found.is_empty() {
        return None;
    }
    let mut pick = None;
    egui::Frame::new().fill(t.checked).corner_radius(4).inner_margin(8).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(strip_label()).color(t.text));
            for f in found.iter().take(MAX_CHIPS) {
                let (label, hint, click) = chip(f);
                let r = ui.small_button(label).on_hover_text(hint);
                if r.clicked() {
                    pick = Some(click.clone());
                }
                let copy = match &click {
                    Pick::Copy(v) => v.clone(),
                    _ => f.value.clone(),
                };
                r.context_menu(|ui| {
                    if ui.button(tr("recognized-copy")).clicked() {
                        pick = Some(Pick::Copy(copy.clone()));
                        ui.close();
                    }
                });
            }
        });
    });
    ui.add_space(8.0);
    pick
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_finding_gets_a_label_and_a_click_that_fits_it() {
        let found = recognize("Ordernummer 4711, totaal € 129,95, betaal op NL91ABNA0417164300. PostNL Track & Trace 3SABCD1234567");
        let chips: Vec<(String, Pick)> = found.iter().map(chip).map(|(label, _, pick)| (label, pick)).collect();
        assert_eq!(
            chips,
            [
                (tr_args("recognized-order", &[("code", "4711".into())]), Pick::Search("\"4711\"".into())),
                ("€ 129,95".to_string(), Pick::Copy("€ 129,95".into())),
                ("IBAN NL91 ABNA 0417 1643 00".to_string(), Pick::Copy("NL91ABNA0417164300".into())),
                ("PostNL 3SABCD1234567".to_string(), Pick::Open("https://jouw.postnl.nl/track-and-trace/3SABCD1234567".into())),
            ]
        );
    }
}
