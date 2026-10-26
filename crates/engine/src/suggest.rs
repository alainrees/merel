//! Suggestions under the search box while a word is being typed: people (contacts first, then
//! whoever sent the mail Merel knows), the search words themselves, folders, accounts and date
//! words. Choosing one puts it in the box in the place of the half-typed word.

use std::collections::HashMap;

use crate::Session;
use crate::search::KEYS;

/// One suggestion: what the row says, a second line, and the search box once it is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    pub text: String,
}

/// The date words the suggestions offer, in Dutch and English.
const DATE_WORDS: [(&str, &str); 8] = [
    ("vandaag", "today"),
    ("gisteren", "yesterday"),
    ("deze-week", "this-week"),
    ("vorige-week", "last-week"),
    ("deze-maand", "this-month"),
    ("vorige-maand", "last-month"),
    ("dit-jaar", "this-year"),
    ("vorig-jaar", "last-year"),
];

/// What `heeft:` takes, in Dutch and English, with what each means.
const HAS_WORDS: [(&str, &str, &str); 6] = [
    ("bijlage", "attachment", "suggest-has-attachment"),
    ("bedrag", "amount", "suggest-has-amount"),
    ("iban", "iban", "suggest-has-iban"),
    ("zending", "tracking", "suggest-has-tracking"),
    ("ordernummer", "order", "suggest-has-order"),
    ("factuurnummer", "invoice", "suggest-has-invoice"),
];

/// What each search word's suggestion says about it.
fn key_detail(key: &str) -> String {
    let id = match key {
        "van:" => "suggest-from",
        "aan:" => "suggest-to",
        "cc:" => "suggest-cc",
        "onderwerp:" => "suggest-subject",
        "datum:" => "suggest-date",
        "voor:" => "suggest-before",
        "na:" => "suggest-after",
        "map:" => "suggest-folder",
        "account:" => "suggest-account",
        "bijlage:" => "suggest-attachment",
        "categorie:" => "suggest-category",
        "heeft:" => "suggest-has",
        "bedrag:" => "suggest-amount",
        "groot:" => "suggest-size",
        _ => "suggest-is",
    };
    merel_i18n::tr(id)
}

/// A value as the box needs it: in quotes when it holds a space.
fn value(v: &str) -> String {
    if v.contains(char::is_whitespace) { format!("\"{v}\"") } else { v.to_string() }
}

impl Session {
    /// Everyone the person might mean: contacts, then senders by how much mail they sent, each
    /// address once, as (name, address).
    fn people(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for c in &self.contacts {
            for e in &c.emails {
                if seen.insert(e.to_lowercase()) {
                    out.push((c.name.clone(), e.clone()));
                }
            }
        }
        let mut senders: HashMap<String, (String, String, usize)> = HashMap::new();
        for m in &self.messages {
            let e = m.from.email().to_lowercase();
            let entry = senders.entry(e).or_insert_with(|| (m.from.display().to_string(), m.from.email().to_string(), 0));
            entry.2 += 1;
        }
        let mut senders: Vec<(String, String, usize)> = senders.into_values().collect();
        senders.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
        for (name, email, _) in senders {
            if seen.insert(email.to_lowercase()) {
                out.push((name, email));
            }
        }
        out
    }

    /// The suggestions for the search box as it stands, at most `limit`. Nothing once the last
    /// word is finished with a space, or while it is a single letter.
    pub fn search_suggestions(&self, text: &str, limit: usize) -> Vec<Suggestion> {
        if text.is_empty() || text.ends_with(char::is_whitespace) {
            return Vec::new();
        }
        let cut = text.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        let (before, word) = text.split_at(cut);
        let lower = word.to_lowercase();
        let negated = lower.starts_with('-');
        let lower = lower.trim_start_matches('-').to_string();
        let lead = if negated { format!("{before}-") } else { before.to_string() };
        let mut out = Vec::new();
        let mut push = |label: String, detail: String, text: String| {
            if !out.iter().any(|s: &Suggestion| s.text == text) {
                out.push(Suggestion { label, detail, text });
            }
        };
        match lower.split_once(':') {
            Some((key, typed)) => {
                let typed = typed.trim_matches('"');
                match key {
                    "van" | "from" | "aan" | "to" | "cc" => {
                        for (name, email) in self.people() {
                            let hay = format!("{name}\n{email}").to_lowercase();
                            if typed.is_empty() || hay.contains(typed) {
                                push(name, email.clone(), format!("{lead}{key}:{email} "));
                            }
                        }
                    }
                    "map" | "folder" => {
                        let mut names: Vec<&str> = self.folders.iter().map(|f| f.name.as_str()).collect();
                        names.sort_unstable();
                        names.dedup();
                        for name in names {
                            if name.to_lowercase().contains(typed) {
                                push(name.to_string(), key_detail("map:"), format!("{lead}{key}:{} ", value(name)));
                            }
                        }
                    }
                    "account" => {
                        for a in &self.accounts {
                            let email = a.address.email();
                            if format!("{}\n{email}", a.name).to_lowercase().contains(typed) {
                                push(a.name.clone(), email.to_string(), format!("{lead}account:{email} "));
                            }
                        }
                    }
                    "datum" | "date" | "voor" | "before" | "na" | "after" => {
                        let english = matches!(key, "date" | "before" | "after");
                        for (nl, en) in DATE_WORDS {
                            let w = if english { en } else { nl };
                            if w.starts_with(typed) {
                                push(format!("{key}:{w}"), key_detail("datum:"), format!("{lead}{key}:{w} "));
                            }
                        }
                    }
                    "is" => {
                        for w in ["ongelezen", "gemarkeerd"] {
                            if w.starts_with(typed) {
                                push(format!("is:{w}"), key_detail("is:"), format!("{lead}is:{w} "));
                            }
                        }
                    }
                    "heeft" | "has" => {
                        let english = key == "has";
                        for (nl, en, id) in HAS_WORDS {
                            let w = if english { en } else { nl };
                            if w.starts_with(typed) {
                                push(format!("{key}:{w}"), merel_i18n::tr(id), format!("{lead}{key}:{w} "));
                            }
                        }
                    }
                    "groot" | "grootte" | "size" => {
                        for w in [">1MB", ">5MB", ">10MB", ">25MB", "<100KB"] {
                            if w.to_lowercase().starts_with(typed) {
                                push(format!("{key}:{w}"), key_detail("groot:"), format!("{lead}{key}:{w} "));
                            }
                        }
                    }
                    "categorie" | "category" => {
                        for c in &self.categories {
                            if c.name.to_lowercase().contains(typed) {
                                push(c.name.clone(), key_detail("categorie:"), format!("{lead}{key}:{} ", value(&c.name)));
                            }
                        }
                    }
                    _ => {}
                }
            }
            None if lower.chars().count() >= 2 => {
                for (nl, en) in KEYS {
                    if nl.starts_with(&lower) || en.starts_with(&lower) {
                        let key = if en.starts_with(&lower) && !nl.starts_with(&lower) { en } else { nl };
                        push(key.to_string(), key_detail(nl), format!("{lead}{key}"));
                    }
                }
                if "heeft:bijlage".starts_with(&lower) || "has:attachment".starts_with(&lower) {
                    push("heeft:bijlage".into(), merel_i18n::tr("suggest-has-attachment"), format!("{lead}heeft:bijlage "));
                }
                if !negated {
                    for (name, email) in self.people() {
                        let hay = format!("{name}\n{email}").to_lowercase();
                        if hay.split(|c: char| !c.is_alphanumeric()).any(|part| part.starts_with(&lower)) {
                            push(name, email.clone(), format!("{lead}van:{email} "));
                        }
                    }
                }
            }
            None => {}
        }
        out.truncate(limit);
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::Session;

    fn texts(s: &Session, text: &str) -> Vec<String> {
        s.search_suggestions(text, 8).into_iter().map(|x| x.text).collect()
    }

    #[test]
    fn people_search_words_folders_and_dates_are_suggested() {
        let s = Session::sample();
        let sender = s.messages[0].from.clone();
        let first = sender.display().split_whitespace().next().unwrap().to_lowercase();
        let typed: String = first.chars().take(3).collect();
        let found = s.search_suggestions(&format!("offerte {typed}"), 8);
        assert!(
            found.iter().any(|x| x.text == format!("offerte van:{} ", sender.email()) && x.detail == sender.email()),
            "a sender by the start of a name: {found:?}"
        );
        assert_eq!(texts(&s, "onder"), ["onderwerp:"], "a search word");
        assert_eq!(texts(&s, "subj"), ["subject:"], "in English too");
        assert!(texts(&s, "da").contains(&"datum:".to_string()));
        assert_eq!(texts(&s, "datum:vo"), ["datum:vorige-week ", "datum:vorige-maand ", "datum:vorig-jaar "]);
        assert_eq!(texts(&s, "date:to"), ["date:today "]);
        let inbox = s.folders[0].name.clone();
        let part: String = inbox.to_lowercase().chars().take(3).collect();
        assert!(texts(&s, &format!("map:{part}")).iter().any(|t| t.starts_with("map:")), "a folder");
        assert_eq!(texts(&s, "is:on"), ["is:ongelezen "]);
        assert_eq!(texts(&s, "heeft:i"), ["heeft:iban "]);
        assert_eq!(texts(&s, "has:tr"), ["has:tracking "]);
        assert!(texts(&s, "bed").contains(&"bedrag:".to_string()));
        assert!(texts(&s, "gro").contains(&"groot:".to_string()));
        assert_eq!(texts(&s, "groot:>1"), ["groot:>1MB ", "groot:>10MB "]);
        assert_eq!(texts(&s, "size:<"), ["size:<100KB "]);
        assert!(texts(&s, "keuken ").is_empty(), "a finished word asks nothing");
        assert!(texts(&s, "k").is_empty(), "one letter is too little");
        assert!(texts(&s, "").is_empty());
        let excluded = texts(&s, "-onder");
        assert_eq!(excluded, ["-onderwerp:"], "a minus stays in front");
    }
}
