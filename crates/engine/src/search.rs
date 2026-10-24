//! Outlook's search words in the search box: `van:sanne`, `aan:jan` (also `cc:`),
//! `onderwerp:offerte`, `heeft:bijlage`, `is:ongelezen` and `is:gemarkeerd` narrow the list by
//! one field or flag, next to plain words that may occur anywhere. More of them narrow by when
//! (`datum:vandaag`, `datum:vorige-week`, `datum:2026-10`, `voor:2026-10-01`, `na:2026`), where
//! (`map:facturen`, `account:werk`), an attachment's name (`bijlage:pdf`) or a category
//! (`categorie:rood`). A phrase in quotes must occur as it stands, a word with a minus in front
//! must not occur at all, and `OF` between two words lets either do. The English forms
//! (`from:`, `to:`, `subject:`, `has:attachment`, `is:unread`, `is:flagged`, `date:`, `before:`,
//! `after:`, `folder:`, `attachment:`, `category:`, `OR`, and `today`, `this-week` and so on)
//! work too. The server gets only the plain words; what it finds is then held to the same
//! operators here.

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use merel_model::{Address, MessageHeader};

/// What the search box says the words mean, for a tooltip.
pub const HINT: &str = "Zoekwoorden zoals in Outlook: van:sanne, aan:jan, onderwerp:offerte, heeft:bijlage, bijlage:pdf, is:ongelezen, is:gemarkeerd, datum:vorige-week, voor:2026-10-01, na:2026, map:facturen, account:werk, categorie:rood. \"Een zin\" tussen aanhalingstekens, -woord sluit uit, OF laat een van beide toe. Losse woorden zoeken overal.";

/// A parsed search: lower-case words plus the operators.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Words that may occur anywhere: subject, sender, recipients or preview; every one must.
    pub words: Vec<String>,
    /// Groups of words joined by `OF`: one word of each group must occur.
    pub either: Vec<Vec<String>>,
    /// Phrases that must occur as they stand.
    pub phrases: Vec<String>,
    /// Words and phrases that must not occur.
    pub not: Vec<String>,
    pub from: Vec<String>,
    /// The recipients, To and Cc alike.
    pub to: Vec<String>,
    pub subject: Vec<String>,
    /// Words in the name of an attachment.
    pub attachment_names: Vec<String>,
    pub categories: Vec<String>,
    /// Words in the name of the folder (`map:`) or of the account (`account:`).
    pub folders: Vec<String>,
    pub accounts: Vec<String>,
    /// From this moment on (seconds since the epoch), inclusive.
    pub after: Option<i64>,
    /// Before this moment, exclusive.
    pub before: Option<i64>,
    pub attachment: bool,
    pub unread: bool,
    pub flagged: bool,
    /// `bedrag:`: an amount in the text or an attachment between these, in cents, inclusive.
    pub amount: Option<(Option<i64>, Option<i64>)>,
    /// `heeft:iban` and the like: kinds of data the index must have recognised.
    pub has: Vec<String>,
}

/// One piece of the search box: what it says, whether it was in quotes, whether a minus led it.
struct Piece {
    text: String,
    quoted: bool,
    negated: bool,
}

/// Splits the box into pieces at spaces, keeping what stands in quotes together, also after a
/// key (`onderwerp:"nieuwe keuken"`).
fn pieces(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&first) = chars.peek() else { break };
        let negated = first == '-';
        if negated {
            chars.next();
        }
        let mut piece = String::new();
        let mut quoted = false;
        let mut in_quotes = false;
        while let Some(&c) = chars.peek() {
            if c == '"' {
                chars.next();
                in_quotes = !in_quotes;
                quoted = true;
                continue;
            }
            if c.is_whitespace() && !in_quotes {
                break;
            }
            piece.push(c);
            chars.next();
        }
        if piece.is_empty() {
            continue;
        }
        out.push(Piece { text: piece, quoted, negated });
    }
    out
}

/// Seconds since the epoch at the start of `day`, on this computer's clock.
fn start_of(day: NaiveDate) -> i64 {
    let midnight = day.and_hms_opt(0, 0, 0).unwrap_or_default();
    Local.from_local_datetime(&midnight).earliest().map(|t| t.timestamp()).unwrap_or_else(|| midnight.and_utc().timestamp())
}

fn first_of_month(year: i32, month: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, 1)
}

fn next_month(day: NaiveDate) -> Option<NaiveDate> {
    if day.month() == 12 { first_of_month(day.year() + 1, 1) } else { first_of_month(day.year(), day.month() + 1) }
}

/// The days a date word stands for, as the first day and the day after the last; `None` for
/// anything that is no date.
pub fn date_range(word: &str, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let week_start = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let month_start = first_of_month(today.year(), today.month())?;
    let year_start = first_of_month(today.year(), 1)?;
    let range = match word {
        "vandaag" | "today" => (today, today + Duration::days(1)),
        "gisteren" | "yesterday" => (today - Duration::days(1), today),
        "deze-week" | "this-week" => (week_start, week_start + Duration::days(7)),
        "vorige-week" | "last-week" => (week_start - Duration::days(7), week_start),
        "deze-maand" | "this-month" => (month_start, next_month(month_start)?),
        "vorige-maand" | "last-month" => {
            let last = month_start - Duration::days(1);
            (first_of_month(last.year(), last.month())?, month_start)
        }
        "dit-jaar" | "this-year" => (year_start, first_of_month(today.year() + 1, 1)?),
        "vorig-jaar" | "last-year" => (first_of_month(today.year() - 1, 1)?, year_start),
        _ => {
            let parts: Vec<&str> = word.split('-').collect();
            let num = |s: &str| s.parse::<u32>().ok();
            match parts.as_slice() {
                [y] if y.len() == 4 => {
                    let y = num(y)? as i32;
                    (first_of_month(y, 1)?, first_of_month(y + 1, 1)?)
                }
                [y, m] if y.len() == 4 => {
                    let start = first_of_month(num(y)? as i32, num(m)?)?;
                    (start, next_month(start)?)
                }
                [y, m, d] if y.len() == 4 => {
                    let day = NaiveDate::from_ymd_opt(num(y)? as i32, num(m)?, num(d)?)?;
                    (day, day + Duration::days(1))
                }
                // Dutch order: 24-10-2026.
                [d, m, y] if y.len() == 4 => {
                    let day = NaiveDate::from_ymd_opt(num(y)? as i32, num(m)?, num(d)?)?;
                    (day, day + Duration::days(1))
                }
                _ => return None,
            }
        }
    };
    Some(range)
}

/// The cents an amount word asks for: `>100`, `>=100`, `<50`, `100-500`, `100..500`, `1250`
/// (any amount from 1250,00 to 1250,99) or `12,50`.
fn amount_range(value: &str) -> Option<(Option<i64>, Option<i64>)> {
    let number = |v: &str| merel_extract::recognize::parse_number(v.trim().trim_start_matches(['€', '$', '£']).trim());
    if let Some(v) = value.strip_prefix(">=") {
        return Some((Some(number(v)?), None));
    }
    if let Some(v) = value.strip_prefix("<=") {
        return Some((None, Some(number(v)?)));
    }
    if let Some(v) = value.strip_prefix('>') {
        return Some((Some(number(v)? + 1), None));
    }
    if let Some(v) = value.strip_prefix('<') {
        return Some((None, Some(number(v)? - 1)));
    }
    if let Some((a, b)) = value.split_once("..").or_else(|| value.split_once('-').filter(|(a, b)| !a.is_empty() && !b.is_empty())) {
        let (a, b) = (number(a)?, number(b)?);
        return Some((Some(a.min(b)), Some(a.max(b))));
    }
    let cents = number(value)?;
    // Whole euros mean any cents; with cents, that amount exactly.
    let whole = !value.contains([',', '.']) || value.ends_with(",-");
    Some((Some(cents), Some(if whole { cents + 99 } else { cents })))
}

impl Query {
    pub fn parse(text: &str) -> Query {
        Query::parse_on(text, Local::now().date_naive())
    }

    /// As [`Query::parse`], with `today` for the date words.
    pub fn parse_on(text: &str, today: NaiveDate) -> Query {
        let mut q = Query::default();
        // A plain word waiting to see whether `OF` follows it.
        let mut last_plain: Option<String> = None;
        let mut join_next = false;
        let flush = |q: &mut Query, word: Option<String>| {
            if let Some(w) = word {
                q.words.push(w);
            }
        };
        for piece in pieces(text) {
            let word = piece.text.to_lowercase();
            // OF with nothing before it to join to is an ordinary word.
            if !piece.quoted && !piece.negated && (piece.text == "OF" || piece.text == "OR") && (last_plain.is_some() || !q.either.is_empty()) {
                join_next = true;
                continue;
            }
            if piece.negated {
                flush(&mut q, last_plain.take());
                q.not.push(word);
                continue;
            }
            if piece.quoted && !word.contains(':') {
                flush(&mut q, last_plain.take());
                if word.contains(char::is_whitespace) {
                    q.phrases.push(word);
                } else {
                    q.words.push(word);
                }
                continue;
            }
            if let Some((key, value)) = word.split_once(':')
                && q.operator(key, value, today)
            {
                flush(&mut q, last_plain.take());
                continue;
            }
            // A plain word (anything else with a colon is one too, "re:" say).
            if join_next {
                join_next = false;
                match last_plain.take() {
                    Some(prev) => q.either.push(vec![prev, word]),
                    None => match q.either.last_mut() {
                        Some(group) => group.push(word),
                        None => q.words.push(word),
                    },
                }
                continue;
            }
            flush(&mut q, last_plain.take());
            last_plain = Some(word);
        }
        flush(&mut q, last_plain);
        q
    }

    /// Takes in `key:value`; false when it is no operator.
    fn operator(&mut self, key: &str, value: &str, today: NaiveDate) -> bool {
        let value = value.trim();
        if value.is_empty() {
            return false;
        }
        let v = value.to_string();
        match key {
            "van" | "from" => self.from.push(v),
            "aan" | "to" | "cc" => self.to.push(v),
            "onderwerp" | "subject" => self.subject.push(v),
            "bijlage" | "attachment" if !matches!(value, "ja" | "yes") => self.attachment_names.push(v),
            "bijlage" | "attachment" => self.attachment = true,
            "heeft" | "has" => {
                let kind = match value {
                    "bijlage" | "bijlagen" | "attachment" | "attachments" => {
                        self.attachment = true;
                        return true;
                    }
                    "bedrag" | "amount" => "amount",
                    "iban" => "iban",
                    "zending" | "pakket" | "trackandtrace" | "track" | "tracking" => "tracking",
                    "ordernummer" | "order" | "bestelling" => "order",
                    "factuurnummer" | "factuur" | "invoice" => "invoice",
                    _ => return false,
                };
                if !self.has.iter().any(|k| k == kind) {
                    self.has.push(kind.to_string());
                }
            }
            "bedrag" | "amount" => {
                let Some((low, high)) = amount_range(value) else { return false };
                let (old_low, old_high) = self.amount.unwrap_or((None, None));
                let low = match (old_low, low) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
                let high = match (old_high, high) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                self.amount = Some((low, high));
            }
            "is" => match value {
                "ongelezen" | "unread" => self.unread = true,
                "gemarkeerd" | "gevlagd" | "flagged" => self.flagged = true,
                _ => return false,
            },
            "categorie" | "category" => self.categories.push(v),
            "map" | "folder" => self.folders.push(v),
            "account" => self.accounts.push(v),
            "datum" | "date" | "voor" | "before" | "na" | "after" => {
                let Some((from, to)) = date_range(value, today) else { return false };
                let (from, to) = (start_of(from), start_of(to));
                match key {
                    "voor" | "before" => self.before = Some(self.before.map_or(from, |b| b.min(from))),
                    "na" | "after" => self.after = Some(self.after.map_or(to, |a| a.max(to))),
                    _ => {
                        self.after = Some(self.after.map_or(from, |a| a.max(from)));
                        self.before = Some(self.before.map_or(to, |b| b.min(to)));
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// Nothing to search for at all.
    pub fn is_empty(&self) -> bool {
        self == &Query::default()
    }

    /// The plain words, for a server that knows no operators.
    pub fn plain(&self) -> String {
        self.words.join(" ")
    }

    /// Whether the message satisfies every operator (the words aside). Folder and account are
    /// the session's to check; see [`crate::Session::search_folders`].
    pub fn keeps(&self, m: &MessageHeader) -> bool {
        let names = |list: &[Address]| {
            let mut h = String::new();
            for a in list {
                h.push_str(a.display());
                h.push('\n');
                h.push_str(a.email());
                h.push('\n');
            }
            h.to_lowercase()
        };
        if !self.from.is_empty() {
            let from = names(std::slice::from_ref(&m.from));
            if !self.from.iter().all(|w| from.contains(w.as_str())) {
                return false;
            }
        }
        if !self.to.is_empty() {
            let to = names(&m.to);
            if !self.to.iter().all(|w| to.contains(w.as_str())) {
                return false;
            }
        }
        if !self.subject.is_empty() {
            let subject = m.subject.to_lowercase();
            if !self.subject.iter().all(|w| subject.contains(w.as_str())) {
                return false;
            }
        }
        if !self.categories.iter().all(|c| m.categories.iter().any(|x| x.to_lowercase().contains(c.as_str()))) {
            return false;
        }
        if self.after.is_some_and(|a| m.date < a) || self.before.is_some_and(|b| m.date >= b) {
            return false;
        }
        // The names of attachments are the index's to check; here only that there are some.
        let attachment = self.attachment || !self.attachment_names.is_empty();
        (!attachment || m.has_attachments) && (!self.unread || !m.flags.seen) && (!self.flagged || m.flags.flagged)
    }

    /// Whether the words hold for what the message list knows of a message (subject, sender,
    /// recipients, preview): every word and phrase occurs, one of each `OF` group, none of the
    /// excluded; and every operator holds.
    pub fn matches(&self, m: &MessageHeader) -> bool {
        // What was recognised in the text only the index knows.
        if !self.keeps(m) || self.amount.is_some() || !self.has.is_empty() {
            return false;
        }
        if self.words.is_empty() && self.either.is_empty() && self.phrases.is_empty() && self.not.is_empty() {
            return true;
        }
        let haystack = {
            let mut h = String::with_capacity(256);
            for part in [m.subject.as_str(), m.from.display(), m.from.email(), m.preview.as_str()] {
                h.push_str(part);
                h.push('\n');
            }
            for a in &m.to {
                h.push_str(a.display());
                h.push('\n');
                h.push_str(a.email());
                h.push('\n');
            }
            h.to_lowercase()
        };
        self.words.iter().chain(&self.phrases).all(|w| haystack.contains(w.as_str()))
            && self.either.iter().all(|group| group.iter().any(|w| haystack.contains(w.as_str())))
            && !self.not.iter().any(|w| haystack.contains(w.as_str()))
    }

    /// Whether there are words for an index to look for, beyond flags, dates, categories and
    /// places, which the session can answer alone.
    pub fn has_words(&self) -> bool {
        !(self.words.is_empty()
            && self.either.is_empty()
            && self.phrases.is_empty()
            && self.from.is_empty()
            && self.to.is_empty()
            && self.subject.is_empty()
            && self.attachment_names.is_empty()
            && self.amount.is_none()
            && self.has.is_empty())
    }
}

/// `text` with `word` added at the end, or, when it holds it already (or one of `aliases`, in
/// any case), with every such word taken out: a search word switched on or off.
pub fn toggle_word(text: &str, word: &str, aliases: &[&str]) -> String {
    let same = |w: &str| w.eq_ignore_ascii_case(word) || aliases.iter().any(|a| w.eq_ignore_ascii_case(a));
    if text.split_whitespace().any(same) {
        let kept: Vec<&str> = text.split_whitespace().filter(|w| !same(w)).collect();
        kept.join(" ")
    } else if text.trim().is_empty() {
        word.to_string()
    } else {
        format!("{} {word}", text.trim_end())
    }
}

/// The keys of the search words in Dutch with their English forms, as the suggestions offer them.
pub const KEYS: [(&str, &str); 14] = [
    ("van:", "from:"),
    ("aan:", "to:"),
    ("cc:", "cc:"),
    ("onderwerp:", "subject:"),
    ("datum:", "date:"),
    ("voor:", "before:"),
    ("na:", "after:"),
    ("map:", "folder:"),
    ("account:", "account:"),
    ("bijlage:", "attachment:"),
    ("categorie:", "category:"),
    ("is:", "is:"),
    ("heeft:", "has:"),
    ("bedrag:", "amount:"),
];

/// `text` with `key:value` in the place of any word with that key (in either language), or
/// added at the end; an empty `value` only takes the key's words out. A value with a space goes
/// in quotes.
pub fn with_operator(text: &str, key: &str, value: &str) -> String {
    let aliases: Vec<&str> = KEYS
        .iter()
        .filter(|(nl, en)| nl.trim_end_matches(':') == key || en.trim_end_matches(':') == key)
        .flat_map(|(nl, en)| [nl.trim_end_matches(':'), en.trim_end_matches(':')])
        .collect();
    let aliases = if aliases.is_empty() { vec![key] } else { aliases };
    // The date words are one family: a new date replaces before and after too.
    let dates = ["datum", "date", "voor", "before", "na", "after"];
    let family: Vec<&str> = if aliases.iter().any(|a| dates.contains(a)) { dates.to_vec() } else { aliases };
    let mut out: Vec<String> = Vec::new();
    for piece in pieces(text) {
        let lower = piece.text.to_lowercase();
        let owned = lower.split_once(':').is_some_and(|(k, _)| family.contains(&k));
        if owned && !piece.negated {
            continue;
        }
        let mut word = String::new();
        if piece.negated {
            word.push('-');
        }
        if piece.quoted {
            match piece.text.split_once(':') {
                Some((k, v)) => word.push_str(&format!("{k}:\"{v}\"")),
                None => word.push_str(&format!("\"{}\"", piece.text)),
            }
        } else {
            word.push_str(&piece.text);
        }
        out.push(word);
    }
    if !value.is_empty() {
        if value.contains(char::is_whitespace) {
            out.push(format!("{key}:\"{value}\""));
        } else {
            out.push(format!("{key}:{value}"));
        }
    }
    let mut s = out.join(" ");
    if !s.is_empty() {
        s.push(' ');
    }
    s
}

/// The value of the first word with `key` (either language) in `text`.
pub fn operator_value(text: &str, key: &str) -> Option<String> {
    let aliases: Vec<&str> = KEYS
        .iter()
        .filter(|(nl, en)| nl.trim_end_matches(':') == key || en.trim_end_matches(':') == key)
        .flat_map(|(nl, en)| [nl.trim_end_matches(':'), en.trim_end_matches(':')])
        .collect();
    pieces(text).into_iter().filter(|p| !p.negated).find_map(|p| {
        let lower = p.text.to_lowercase();
        let (k, v) = lower.split_once(':')?;
        (aliases.contains(&k) || k == key).then(|| v.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn operators_are_read_in_dutch_and_english_and_the_rest_stays_plain() {
        let q = Query::parse(" Van:Sanne aan:jan onderwerp:\"offerte\" heeft:bijlage is:ongelezen is:gemarkeerd Re: planning ");
        assert_eq!(q.from, ["sanne"]);
        assert_eq!(q.to, ["jan"]);
        assert_eq!(q.subject, ["offerte"]);
        assert!(q.attachment && q.unread && q.flagged);
        assert_eq!(q.words, ["re:", "planning"], "a colon without a known key is just a word");
        assert_eq!(q.plain(), "re: planning");
        let en = Query::parse("from:a to:b subject:c has:attachment is:unread is:flagged");
        assert_eq!(
            (en.from, en.to, en.subject, en.attachment, en.unread, en.flagged),
            (vec!["a".into()], vec!["b".into()], vec!["c".into()], true, true, true)
        );
        assert!(Query::parse("  ").is_empty());
        assert!(!Query::parse("is:ongelezen").is_empty());
        assert_eq!(Query::parse("van:").words, ["van:"], "an empty value is no operator");
        assert_eq!(Query::parse("cc:piet").to, ["piet"], "cc counts as a recipient");
        assert_eq!(Query::parse("bijlage:pdf").attachment_names, ["pdf"]);
        assert_eq!(Query::parse("map:Facturen account:werk categorie:rood").folders, ["facturen"]);
        assert_eq!(Query::parse("map:x account:werk").accounts, ["werk"]);
        assert_eq!(Query::parse("categorie:rood").categories, ["rood"]);
    }

    #[test]
    fn phrases_exclusions_and_either_are_read() {
        let q = Query::parse("\"nieuwe keuken\" -offerte aannemer OF loodgieter onderwerp:\"plan b\" -\"oude zaak\"");
        assert_eq!(q.phrases, ["nieuwe keuken"]);
        assert_eq!(q.not, ["offerte", "oude zaak"]);
        assert_eq!(q.either, [vec!["aannemer".to_string(), "loodgieter".to_string()]]);
        assert_eq!(q.subject, ["plan b"], "quotes after a key keep the value together");
        assert!(q.words.is_empty());
        let three = Query::parse("a OR b OF c d");
        assert_eq!(three.either, [vec!["a".to_string(), "b".into(), "c".into()]]);
        assert_eq!(three.words, ["d"]);
        assert_eq!(Query::parse("of koffie").words, ["of", "koffie"], "a lower-case of is a Dutch word");
        assert_eq!(Query::parse("OF koffie").words, ["of", "koffie"], "nothing to join to");
    }

    #[test]
    fn amounts_and_recognised_kinds_are_read_and_left_to_the_index() {
        assert_eq!(Query::parse("bedrag:>100").amount, Some((Some(10_001), None)));
        assert_eq!(Query::parse("amount:>=100").amount, Some((Some(10_000), None)));
        assert_eq!(Query::parse("bedrag:<50").amount, Some((None, Some(4_999))));
        assert_eq!(Query::parse("bedrag:100-500").amount, Some((Some(10_000), Some(50_000))));
        assert_eq!(Query::parse("bedrag:500..100").amount, Some((Some(10_000), Some(50_000))));
        assert_eq!(Query::parse("bedrag:1250").amount, Some((Some(125_000), Some(125_099))), "whole euros: any cents");
        assert_eq!(Query::parse("bedrag:12,50").amount, Some((Some(1_250), Some(1_250))));
        assert_eq!(Query::parse("bedrag:€1.250,00").amount, Some((Some(125_000), Some(125_000))));
        assert_eq!(Query::parse("bedrag:>100 bedrag:<500").amount, Some((Some(10_001), Some(49_999))), "two narrow together");
        assert_eq!(Query::parse("bedrag:veel").words, ["bedrag:veel"], "no amount: a plain word");
        let q = Query::parse("heeft:iban has:tracking heeft:ordernummer heeft:factuur heeft:bedrag heeft:iban");
        assert_eq!(q.has, ["iban", "tracking", "order", "invoice", "amount"]);
        assert!(q.has_words(), "only the index can answer");
        let m = crate::Session::sample().messages.first().cloned().expect("sample mail");
        assert!(!q.matches(&m) && q.keeps(&m), "a listed message waits for the index");
    }

    #[test]
    fn date_words_cover_the_days_they_name() {
        let today = day(2026, 10, 24); // a Saturday
        assert_eq!(date_range("vandaag", today), Some((today, day(2026, 10, 25))));
        assert_eq!(date_range("yesterday", today), Some((day(2026, 10, 23), today)));
        assert_eq!(date_range("deze-week", today), Some((day(2026, 10, 19), day(2026, 10, 26))));
        assert_eq!(date_range("vorige-week", today), Some((day(2026, 10, 12), day(2026, 10, 19))));
        assert_eq!(date_range("deze-maand", today), Some((day(2026, 10, 1), day(2026, 11, 1))));
        assert_eq!(date_range("vorige-maand", day(2026, 1, 5)), Some((day(2025, 12, 1), day(2026, 1, 1))));
        assert_eq!(date_range("dit-jaar", today), Some((day(2026, 1, 1), day(2027, 1, 1))));
        assert_eq!(date_range("2025", today), Some((day(2025, 1, 1), day(2026, 1, 1))));
        assert_eq!(date_range("2026-12", today), Some((day(2026, 12, 1), day(2027, 1, 1))));
        assert_eq!(date_range("2026-10-01", today), Some((day(2026, 10, 1), day(2026, 10, 2))));
        assert_eq!(date_range("01-10-2026", today), Some((day(2026, 10, 1), day(2026, 10, 2))));
        assert_eq!(date_range("morgen-ochtend", today), None);
        assert_eq!(date_range("2026-13", today), None);
        let q = Query::parse_on("datum:deze-maand", today);
        assert_eq!((q.after, q.before), (Some(start_of(day(2026, 10, 1))), Some(start_of(day(2026, 11, 1)))));
        let q = Query::parse_on("na:2025 voor:2026-10-01", today);
        assert_eq!((q.after, q.before), (Some(start_of(day(2026, 1, 1))), Some(start_of(day(2026, 10, 1)))));
        assert_eq!(Query::parse_on("datum:ooit", today).words, ["datum:ooit"], "no date: a plain word");
    }

    #[test]
    fn operators_hold_a_message_to_one_field_and_plain_words_to_all() {
        let s = crate::Session::sample();
        let mut m = s.messages.first().cloned().expect("sample mail");
        m.from = Address::new(Some("Sanne de Vries"), "sanne@example.org").expect("address");
        m.to = vec![Address::new(Some("Jan"), "jan@example.org").expect("address")];
        m.subject = "Offerte drukwerk".into();
        m.preview = "zie bijlage voor de nieuwe keuken".into();
        m.has_attachments = false;
        m.flags.seen = true;
        m.flags.flagged = false;
        m.categories = vec!["Rood".into()];
        m.date = start_of(day(2026, 10, 20)) + 3600;
        let today = day(2026, 10, 24);
        let q = |t: &str| Query::parse_on(t, today);
        assert!(q("van:sanne").matches(&m));
        assert!(q("van:vries aan:jan onderwerp:drukwerk").matches(&m));
        assert!(!q("van:jan").matches(&m), "the recipient is not the sender");
        assert!(!q("onderwerp:bijlage").matches(&m), "the preview is not the subject");
        assert!(q("bijlage").matches(&m), "a plain word looks everywhere");
        assert!(!q("heeft:bijlage").matches(&m));
        assert!(!q("bijlage:pdf").matches(&m), "no attachments at all");
        assert!(!q("is:ongelezen").matches(&m));
        assert!(!q("is:gemarkeerd").matches(&m));
        assert!(q("\"nieuwe keuken\"").matches(&m) && !q("\"keuken nieuwe\"").matches(&m));
        assert!(q("drukwerk -factuur").matches(&m) && !q("drukwerk -keuken").matches(&m));
        assert!(q("factuur OF offerte").matches(&m) && !q("factuur OF nota").matches(&m));
        assert!(q("categorie:rood").matches(&m) && !q("categorie:blauw").matches(&m));
        assert!(q("datum:deze-week").matches(&m) && !q("datum:vorige-week").matches(&m));
        assert!(q("na:2026-10-19").matches(&m) && !q("na:2026-10-20").matches(&m));
        assert!(q("voor:2026-10-21").matches(&m) && !q("voor:2026-10-20").matches(&m));
        m.has_attachments = true;
        m.flags.seen = false;
        m.flags.flagged = true;
        assert!(q("heeft:bijlage is:ongelezen is:gemarkeerd van:sanne offerte").matches(&m));
        assert!(!q("van:sanne zzz").matches(&m), "every plain word must occur");
        assert!(q("van:sanne").keeps(&m) && !q("van:piet").keeps(&m));
    }

    #[test]
    fn an_operator_is_put_in_or_replaced_in_the_box() {
        assert_eq!(with_operator("offerte", "datum", "vandaag"), "offerte datum:vandaag ");
        assert_eq!(with_operator("offerte date:2025 na:2024", "datum", "deze-week"), "offerte datum:deze-week ", "one date family");
        assert_eq!(with_operator("from:jan keuken", "van", "sanne@x.nl"), "keuken van:sanne@x.nl ");
        assert_eq!(with_operator("keuken map:inbox", "map", "Klant A"), "keuken map:\"Klant A\" ");
        assert_eq!(with_operator("keuken datum:vandaag", "datum", ""), "keuken ", "an empty value takes it out");
        assert_eq!(with_operator("\"nieuwe keuken\" -offerte", "van", "x"), "\"nieuwe keuken\" -offerte van:x ");
        assert_eq!(operator_value("keuken from:jan", "van").as_deref(), Some("jan"));
        assert_eq!(operator_value("keuken", "van"), None);
    }
}
