//! Data recognised in the text of a message or attachment, by fixed rules and without AI:
//! amounts of money, IBANs (checked with their mod-97 check digits), track-and-trace codes,
//! order numbers and invoice numbers. The index keeps them so a search can ask for
//! `bedrag:>100` or `heeft:iban`, and the reading pane shows them as chips.

use std::ops::Range;

/// The most text that is looked through, in bytes.
const MAX_SCAN: usize = 256 * 1024;
/// The most findings one text gives.
const MAX_FOUND: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Currency {
    Euro,
    Dollar,
    Pound,
}

impl Currency {
    pub fn symbol(self) -> &'static str {
        match self {
            Currency::Euro => "€",
            Currency::Dollar => "$",
            Currency::Pound => "£",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Carrier {
    PostNl,
    Dhl,
    Dpd,
    Ups,
    Gls,
}

impl Carrier {
    pub fn name(self) -> &'static str {
        match self {
            Carrier::PostNl => "PostNL",
            Carrier::Dhl => "DHL",
            Carrier::Dpd => "DPD",
            Carrier::Ups => "UPS",
            Carrier::Gls => "GLS",
        }
    }

    /// The carrier's own page for a code. Only ever opened when the person clicks.
    pub fn link(self, code: &str) -> String {
        let code: String = code.chars().filter(char::is_ascii_alphanumeric).collect();
        match self {
            Carrier::PostNl => format!("https://jouw.postnl.nl/track-and-trace/{code}"),
            Carrier::Dhl => format!("https://www.dhl.com/nl-nl/home/tracking.html?tracking-id={code}"),
            Carrier::Dpd => format!("https://tracking.dpd.de/status/nl_NL/parcel/{code}"),
            Carrier::Ups => format!("https://www.ups.com/track?tracknum={code}"),
            Carrier::Gls => format!("https://gls-group.com/NL/nl/volg-je-pakket?match={code}"),
        }
    }
}

/// What was recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum What {
    /// An amount in cents.
    Amount {
        cents: i64,
        currency: Currency,
    },
    Iban,
    Tracking {
        carrier: Option<Carrier>,
    },
    Order,
    Invoice,
}

impl What {
    /// The name the index and the search words use for this kind.
    pub fn kind(self) -> &'static str {
        match self {
            What::Amount { .. } => "amount",
            What::Iban => "iban",
            What::Tracking { .. } => "tracking",
            What::Order => "order",
            What::Invoice => "invoice",
        }
    }
}

/// One finding: what it is, the value as the index keeps it (an IBAN without spaces, say) and
/// where it stands in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub what: What,
    pub value: String,
    pub range: Range<usize>,
}

/// A word of the text with where it stands, its punctuation around it taken off.
#[derive(Clone, Debug)]
struct Word<'a> {
    text: &'a str,
    range: Range<usize>,
}

fn words(text: &str) -> Vec<Word<'_>> {
    let mut out = Vec::new();
    let mut start = None;
    let mut push = |s: usize, e: usize| {
        let raw = &text[s..e];
        let lead = raw.len() - raw.trim_start_matches(|c: char| "([{\"'«“‘<#".contains(c)).len();
        let body = &raw[lead..];
        let mut trimmed = body.trim_end_matches(|c: char| ".,;:!?)]}\"'»”’>".contains(c));
        // "12,-" keeps its dash, so the amount reads.
        if body[trimmed.len()..].starts_with(",-") {
            trimmed = &body[..trimmed.len() + 2];
        }
        if !trimmed.is_empty() {
            out.push(Word { text: trimmed, range: s + lead..s + lead + trimmed.len() });
        }
    };
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                push(s, i);
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        push(s, text.len());
    }
    out
}

/// Everything recognised in `text`, each kind and value once, in the order they stand.
pub fn recognize(text: &str) -> Vec<Found> {
    let mut end = text.len().min(MAX_SCAN);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let text = &text[..end];
    let words = words(text);
    let lower: Vec<String> = words.iter().map(|w| w.text.to_lowercase()).collect();
    let carrier = carrier_named(&lower);
    let mut out: Vec<Found> = Vec::new();
    let mut taken = vec![false; words.len()];
    let push = |out: &mut Vec<Found>, found: Found| {
        if out.len() < MAX_FOUND && !out.iter().any(|f| f.what.kind() == found.what.kind() && f.value.eq_ignore_ascii_case(&found.value)) {
            out.push(found);
        }
    };
    for i in 0..words.len() {
        if taken[i] {
            continue;
        }
        if let Some((found, used)) = iban_at(&words, i) {
            taken[i..i + used].iter_mut().for_each(|t| *t = true);
            push(&mut out, found);
            continue;
        }
        if let Some(found) = amount_at(&words, &lower, i) {
            taken[i] = true;
            push(&mut out, found);
            continue;
        }
        if let Some(c) = tracking_pattern(words[i].text) {
            taken[i] = true;
            push(&mut out, Found { what: What::Tracking { carrier: Some(c) }, value: words[i].text.to_uppercase(), range: words[i].range.clone() });
            continue;
        }
        if let Some((what, at)) = labelled_at(&lower, i, carrier) {
            if taken[at] {
                continue;
            }
            let w = &words[at];
            let value = if matches!(what, What::Tracking { .. }) { w.text.to_uppercase() } else { w.text.to_string() };
            taken[at] = true;
            push(&mut out, Found { what, value, range: w.range.clone() });
        }
    }
    out.sort_by_key(|f| f.range.start);
    out
}

/// The carrier the text names, if any.
fn carrier_named(lower: &[String]) -> Option<Carrier> {
    lower.iter().find_map(|w| match w.as_str() {
        "postnl" => Some(Carrier::PostNl),
        "dhl" => Some(Carrier::Dhl),
        "dpd" => Some(Carrier::Dpd),
        "ups" => Some(Carrier::Ups),
        "gls" => Some(Carrier::Gls),
        _ => None,
    })
}

/// A code whose shape alone tells the carrier.
fn tracking_pattern(word: &str) -> Option<Carrier> {
    let w = word.to_ascii_uppercase();
    if !w.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let digits = w.bytes().filter(u8::is_ascii_digit).count();
    if w.starts_with("3S") && (13..=15).contains(&w.len()) && digits >= 6 {
        return Some(Carrier::PostNl);
    }
    if w.starts_with("1Z") && w.len() == 18 && digits >= 6 {
        return Some(Carrier::Ups);
    }
    if (w.starts_with("JVGL") || w.starts_with("JJD")) && (14..=26).contains(&w.len()) && digits >= 8 {
        return Some(Carrier::Dhl);
    }
    None
}

/// Words that say a number follows, and what kind of number.
fn label(word: &str) -> Option<What> {
    let tracking = What::Tracking { carrier: None };
    Some(match word.trim_end_matches(['#', ':']) {
        "ordernummer" | "ordernr" | "bestelnummer" | "bestelnr" | "order" | "bestelling" | "ordernumber" | "order-id" | "orderid" => What::Order,
        "factuurnummer" | "factuurnr" | "factuur" | "invoice" | "invoicenumber" | "rekening" => What::Invoice,
        "track" | "tracking" | "trackingnummer" | "trackingcode" | "trackingnumber" | "zendingnummer" | "zendingsnummer" | "pakketnummer"
        | "volgcode" | "barcode" | "verzendcode" | "sendungsnummer" | "track&trace" | "track-and-trace" | "t&t" => tracking,
        _ => return None,
    })
}

/// Words between a label and its number: "nummer", "nr.", "#", "is", "&", "trace".
fn filler(word: &str) -> bool {
    matches!(
        word,
        "nummer"
            | "number"
            | "nr"
            | "nr."
            | "no"
            | "no."
            | "#"
            | ":"
            | "is"
            | "&"
            | "and"
            | "trace"
            | "code"
            | "id"
            | "-"
            | "van"
            | "je"
            | "uw"
            | "your"
    )
}

/// A label at `i` and, a few words on, the number it introduces: (what, index of that word).
fn labelled_at(lower: &[String], i: usize, carrier: Option<Carrier>) -> Option<(What, usize)> {
    let mut what = label(&lower[i])?;
    if let What::Tracking { .. } = what {
        what = What::Tracking { carrier };
    }
    let mut at = i + 1;
    while at < lower.len() && at <= i + 4 && filler(lower[at].trim_end_matches(':')) {
        at += 1;
    }
    let word = lower.get(at)?;
    let ok = match what {
        What::Tracking { .. } => {
            (8..=35).contains(&word.len()) && word.bytes().all(|b| b.is_ascii_alphanumeric()) && word.bytes().filter(u8::is_ascii_digit).count() >= 6
        }
        _ => {
            (3..=30).contains(&word.len())
                && word.bytes().any(|b| b.is_ascii_digit())
                && word.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_/.".contains(&b))
                // A date or an amount is no number of this kind.
                && !word.contains(',')
                && !is_date(word)
        }
    };
    ok.then_some((what, at))
}

/// "24-10-2026", "2026/10/24", "24.10.26": three groups of digits.
fn is_date(word: &str) -> bool {
    let parts: Vec<&str> = word.split(['-', '/', '.']).collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()))
}

fn currency_of(word: &str) -> Option<Currency> {
    match word {
        "€" | "eur" | "euro" => Some(Currency::Euro),
        "$" | "usd" => Some(Currency::Dollar),
        "£" | "gbp" => Some(Currency::Pound),
        _ => None,
    }
}

/// An amount at word `i`: "€ 1.250,00", "€12,-", "12,50 EUR", "EUR 99.95", "45,00€".
fn amount_at(words: &[Word<'_>], lower: &[String], i: usize) -> Option<Found> {
    let w = lower[i].as_str();
    let found =
        |cents: i64, currency: Currency, range: Range<usize>| Some(Found { what: What::Amount { cents, currency }, value: cents.to_string(), range });
    for sym in ["€", "$", "£", "eur", "usd", "gbp"] {
        if let Some(rest) = w.strip_prefix(sym)
            && !rest.is_empty()
            && let Some(cents) = parse_number(rest)
        {
            return found(cents, currency_of(sym)?, words[i].range.clone());
        }
        if let Some(rest) = w.strip_suffix(sym)
            && !rest.is_empty()
            && let Some(cents) = parse_number(rest)
        {
            return found(cents, currency_of(sym)?, words[i].range.clone());
        }
    }
    if let Some(currency) = currency_of(w)
        && let Some(cents) = lower.get(i + 1).and_then(|n| parse_number(n))
    {
        return found(cents, currency, words[i].range.start..words[i + 1].range.end);
    }
    if let Some(cents) = parse_number(w)
        && let Some(currency) = lower.get(i + 1).and_then(|c| currency_of(c))
    {
        return found(cents, currency, words[i].range.start..words[i + 1].range.end);
    }
    None
}

/// A number as amounts are written, in cents: "1.250,00", "1,250.00", "1250", "12,5", "12,-".
pub fn parse_number(s: &str) -> Option<i64> {
    let s = s.trim_end_matches(".-").trim_end_matches(",-").trim_end_matches(",--");
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b',') || !s.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let (whole, cents) = match s.rfind(['.', ',']) {
        Some(at) if (1..=2).contains(&(s.len() - at - 1)) => {
            let frac = &s[at + 1..];
            let cents: i64 = frac.parse().ok()?;
            (&s[..at], if frac.len() == 1 { cents * 10 } else { cents })
        }
        _ => (s, 0),
    };
    // What is left uses one separator, between groups of three.
    let seps: Vec<char> = whole.chars().filter(|c| *c == '.' || *c == ',').collect();
    if seps.windows(2).any(|p| p[0] != p[1]) {
        return None;
    }
    if !seps.is_empty() {
        let groups: Vec<&str> = whole.split(['.', ',']).collect();
        if groups[0].is_empty() || groups[0].len() > 3 || groups[1..].iter().any(|g| g.len() != 3) {
            return None;
        }
    }
    let digits: String = whole.chars().filter(char::is_ascii_digit).collect();
    if digits.len() > 10 {
        return None;
    }
    let whole: i64 = digits.parse().ok()?;
    Some(whole * 100 + cents)
}

/// The length of an IBAN in each country that uses them.
fn iban_length(country: &str) -> Option<usize> {
    Some(match country {
        "NO" => 15,
        "BE" => 16,
        "DK" | "FI" | "FO" | "GL" | "NL" => 18,
        "MK" | "SI" => 19,
        "AT" | "BA" | "EE" | "KZ" | "LT" | "LU" | "XK" => 20,
        "CH" | "CR" | "HR" | "LI" | "LV" => 21,
        "BG" | "BH" | "DE" | "GB" | "GE" | "IE" | "ME" | "RS" | "VA" => 22,
        "AE" | "GI" | "IL" | "TL" => 23,
        "AD" | "CZ" | "ES" | "MD" | "PK" | "RO" | "SA" | "SE" | "SK" | "TN" | "VG" => 24,
        "PT" | "ST" => 25,
        "IS" | "TR" => 26,
        "FR" | "GR" | "IT" | "MC" | "MR" | "SM" => 27,
        "AL" | "AZ" | "BY" | "CY" | "DO" | "GT" | "HU" | "LB" | "PL" => 28,
        "BR" | "EG" | "PS" | "QA" | "UA" => 29,
        "JO" | "KW" | "MU" => 30,
        "MT" | "SC" => 31,
        _ => return None,
    })
}

/// Whether `iban` (upper case, no spaces) passes the mod-97 check.
fn iban_checks(iban: &str) -> bool {
    let (head, tail) = iban.split_at(4);
    let mut rest: u32 = 0;
    for c in tail.chars().chain(head.chars()) {
        let v = match c.to_digit(36) {
            Some(v) => v,
            None => return false,
        };
        rest = if v >= 10 { (rest * 100 + v) % 97 } else { (rest * 10 + v) % 97 };
    }
    rest == 1
}

/// An IBAN starting at word `i`, written whole or in groups: the finding and how many words it took.
fn iban_at(words: &[Word<'_>], i: usize) -> Option<(Found, usize)> {
    let first = words[i].text.to_ascii_uppercase();
    let b = first.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_uppercase() || !b[1].is_ascii_uppercase() || !b[2].is_ascii_digit() || !b[3].is_ascii_digit() {
        return None;
    }
    let length = iban_length(&first[..2])?;
    let mut compact = String::new();
    for (n, w) in words[i..].iter().enumerate().take(9) {
        let part = w.text.to_ascii_uppercase();
        if !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        compact.push_str(&part);
        if compact.len() == length {
            if !iban_checks(&compact) {
                return None;
            }
            let range = words[i].range.start..w.range.end;
            return Some((Found { what: What::Iban, value: compact, range }, n + 1));
        }
        if compact.len() > length {
            return None;
        }
    }
    None
}

/// An amount as Dutch writes it: "€ 1.250,00".
pub fn format_amount(cents: i64, currency: Currency) -> String {
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (n, c) in whole.chars().enumerate() {
        if n > 0 && (whole.len() - n).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(c);
    }
    format!("{} {grouped},{:02}", currency.symbol(), cents % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(String, String)> {
        recognize(text).into_iter().map(|f| (f.what.kind().to_string(), f.value)).collect()
    }

    #[test]
    fn amounts_in_every_way_they_are_written() {
        assert_eq!(parse_number("1.250,00"), Some(125_000));
        assert_eq!(parse_number("1,250.00"), Some(125_000));
        assert_eq!(parse_number("1250"), Some(125_000));
        assert_eq!(parse_number("12,5"), Some(1_250));
        assert_eq!(parse_number("12,-"), Some(1_200));
        assert_eq!(parse_number("1.250.000"), Some(125_000_000));
        assert_eq!(parse_number("1.25.0"), None);
        assert_eq!(parse_number("1,250.000"), None);
        let found = kinds("Totaal € 1.250,00, daarvan btw €216,94. Verzending 4,95 EUR en EUR 12,- korting; ook $ 99.95 of 45,00€.");
        let cents: Vec<&str> = found.iter().filter(|(k, _)| k == "amount").map(|(_, v)| v.as_str()).collect();
        assert_eq!(cents, ["125000", "21694", "495", "1200", "9995", "4500"]);
        assert!(kinds("Bel 06 1234 5678 of kom op 12-10 langs, huisnummer 125").iter().all(|(k, _)| k != "amount"), "no currency, no amount");
        assert_eq!(format_amount(125_000, Currency::Euro), "€ 1.250,00");
        assert_eq!(format_amount(495, Currency::Euro), "€ 4,95");
    }

    #[test]
    fn an_iban_counts_only_with_correct_check_digits() {
        assert_eq!(kinds("Graag overmaken naar NL91 ABNA 0417 1643 00 t.n.v. Merel."), [("iban".to_string(), "NL91ABNA0417164300".to_string())]);
        assert_eq!(kinds("IBAN: DE89370400440532013000"), [("iban".to_string(), "DE89370400440532013000".to_string())]);
        assert!(kinds("NL92 ABNA 0417 1643 00").is_empty(), "a wrong check digit");
        assert!(kinds("NL91 ABNA 0417").is_empty(), "too short");
    }

    #[test]
    fn track_and_trace_codes_by_shape_or_by_the_words_before_them() {
        let found = recognize("Je pakket is onderweg met PostNL. Track & Trace: 3SABCD1234567. UPS: 1Z999AA10123456784");
        let codes: Vec<(What, &str)> = found.iter().map(|f| (f.what, f.value.as_str())).collect();
        assert_eq!(
            codes,
            [
                (What::Tracking { carrier: Some(Carrier::PostNl) }, "3SABCD1234567"),
                (What::Tracking { carrier: Some(Carrier::Ups) }, "1Z999AA10123456784")
            ]
        );
        let found = recognize("Verzonden met DPD, trackingnummer 01234567890123.");
        assert_eq!(found[0].what, What::Tracking { carrier: Some(Carrier::Dpd) });
        assert_eq!(found[0].value, "01234567890123");
        assert_eq!(Carrier::Ups.link("1Z999AA10123456784"), "https://www.ups.com/track?tracknum=1Z999AA10123456784");
    }

    #[test]
    fn order_and_invoice_numbers_after_their_label() {
        assert_eq!(
            kinds("Bedankt voor je bestelling! Ordernummer: 4711-2026. Factuurnummer F2026/118, betaal voor 1 november."),
            [("order".to_string(), "4711-2026".to_string()), ("invoice".to_string(), "F2026/118".to_string())]
        );
        assert_eq!(kinds("Your order #123456789 has shipped"), [("order".to_string(), "123456789".to_string())]);
        assert_eq!(kinds("Invoice no. INV-0042"), [("invoice".to_string(), "INV-0042".to_string())]);
        assert!(kinds("De bestelling is binnen").is_empty(), "a label without a number");
        assert!(kinds("factuur van 12,50 euro").iter().all(|(k, _)| k != "invoice"), "an amount is no invoice number");
        assert!(kinds("de factuur 24-10-2026 komt").is_empty(), "nor a date");
        let text = "Ordernummer 4711";
        let f = &recognize(text)[0];
        assert_eq!(&text[f.range.clone()], "4711");
    }
}
