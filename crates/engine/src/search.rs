//! Outlook's search words in the search box: `van:sanne`, `aan:jan`, `onderwerp:offerte`,
//! `heeft:bijlage`, `is:ongelezen` and `is:gemarkeerd` narrow the list by one field or flag,
//! next to plain words that may occur anywhere. The English forms (`from:`, `to:`, `subject:`,
//! `has:attachment`, `is:unread`, `is:flagged`) work too. The server gets only the plain
//! words; what it finds is then held to the same operators here.

use merel_model::{Address, MessageHeader};

/// What the search box says the words mean, for a tooltip.
pub const HINT: &str =
    "Zoekwoorden zoals in Outlook: van:sanne, aan:jan, onderwerp:offerte, heeft:bijlage, is:ongelezen, is:gemarkeerd. Losse woorden zoeken overal.";

/// A parsed search: lower-case plain words plus the operators.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Words that may occur anywhere: subject, sender, recipients or preview.
    pub words: Vec<String>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub subject: Vec<String>,
    pub attachment: bool,
    pub unread: bool,
    pub flagged: bool,
}

impl Query {
    pub fn parse(text: &str) -> Query {
        let mut q = Query::default();
        for raw in text.split_whitespace() {
            let word = raw.trim_matches('"').to_lowercase();
            if word.is_empty() {
                continue;
            }
            let Some((key, value)) = word.split_once(':') else {
                q.words.push(word);
                continue;
            };
            let value = value.trim_matches('"');
            match (key, value) {
                ("van" | "from", v) if !v.is_empty() => q.from.push(v.to_string()),
                ("aan" | "to", v) if !v.is_empty() => q.to.push(v.to_string()),
                ("onderwerp" | "subject", v) if !v.is_empty() => q.subject.push(v.to_string()),
                ("heeft" | "has", "bijlage" | "bijlagen" | "attachment" | "attachments") => q.attachment = true,
                ("is", "ongelezen" | "unread") => q.unread = true,
                ("is", "gemarkeerd" | "gevlagd" | "flagged") => q.flagged = true,
                // Anything else with a colon is an ordinary word, "re:" say.
                _ => q.words.push(word),
            }
        }
        q
    }

    /// Nothing to search for at all.
    pub fn is_empty(&self) -> bool {
        self == &Query::default()
    }

    /// The plain words, for a server that knows no operators.
    pub fn plain(&self) -> String {
        self.words.join(" ")
    }

    /// Whether the message satisfies every operator (the plain words aside).
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
        (!self.attachment || m.has_attachments) && (!self.unread || !m.flags.seen) && (!self.flagged || m.flags.flagged)
    }

    /// Whether every plain word occurs in the subject, sender, recipients or preview, and every
    /// operator holds.
    pub fn matches(&self, m: &MessageHeader) -> bool {
        if !self.keeps(m) {
            return false;
        }
        if self.words.is_empty() {
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
        self.words.iter().all(|w| haystack.contains(w.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn operators_hold_a_message_to_one_field_and_plain_words_to_all() {
        let s = crate::Session::sample();
        let mut m = s.messages.first().cloned().expect("sample mail");
        m.from = Address::new(Some("Sanne de Vries"), "sanne@example.org").expect("address");
        m.to = vec![Address::new(Some("Jan"), "jan@example.org").expect("address")];
        m.subject = "Offerte drukwerk".into();
        m.preview = "zie bijlage".into();
        m.has_attachments = false;
        m.flags.seen = true;
        m.flags.flagged = false;
        assert!(Query::parse("van:sanne").matches(&m));
        assert!(Query::parse("van:vries aan:jan onderwerp:drukwerk").matches(&m));
        assert!(!Query::parse("van:jan").matches(&m), "the recipient is not the sender");
        assert!(!Query::parse("onderwerp:bijlage").matches(&m), "the preview is not the subject");
        assert!(Query::parse("bijlage").matches(&m), "a plain word looks everywhere");
        assert!(!Query::parse("heeft:bijlage").matches(&m));
        assert!(!Query::parse("is:ongelezen").matches(&m));
        assert!(!Query::parse("is:gemarkeerd").matches(&m));
        m.has_attachments = true;
        m.flags.seen = false;
        m.flags.flagged = true;
        assert!(Query::parse("heeft:bijlage is:ongelezen is:gemarkeerd van:sanne offerte").matches(&m));
        assert!(!Query::parse("van:sanne zzz").matches(&m), "every plain word must occur");
        assert!(Query::parse("van:sanne").keeps(&m) && !Query::parse("van:piet").keeps(&m));
    }
}
