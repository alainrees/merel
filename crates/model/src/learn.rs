//! The learning junk filter: it learns from what the person files as junk, and takes back
//! out of the Junk folder, which words make mail junk, and files new mail that looks the same.
//! Everything stays on this computer, in `junk-learned.json`. The file holds no readable
//! words: every token is a salted hash, with a salt drawn once per installation, so the hashes
//! cannot be looked up in a word list either.
//!
//! The sum is Gary Robinson's, as SpamBayes does it: a probability per token from how often it
//! was in junk and in good mail, the clearest tokens combined with Fisher's chi-squared method.
//! That gives a score between 0 (good) and 1 (junk) and stays near 0.5 when unsure.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{Address, MessageHeader};

/// From this score on a message counts as junk.
pub const JUNK_SCORE: f64 = 0.9;
/// How many junk and how many good messages the filter needs before it files anything.
pub const MIN_TRAINED: u32 = 10;
/// The most tokens kept; past it the rarest go.
const MAX_TOKENS: usize = 60_000;
/// The most messages remembered, so a change of mind can be taken back.
const MAX_SEEN: usize = 5_000;
/// The most tokens one message counts with.
const MAX_MESSAGE_TOKENS: usize = 400;
/// How many of the clearest tokens decide.
const MAX_DECIDING: usize = 150;
/// Robinson's strength of the prior and the prior itself, for tokens seen only a few times.
const PRIOR_STRENGTH: f64 = 0.45;
const PRIOR: f64 = 0.5;

/// What the filter has learned.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Learned {
    /// Drawn once per installation; 0 while nothing has been learned.
    #[serde(default)]
    pub salt: u64,
    /// How many junk and how many good messages it learned from.
    #[serde(default)]
    pub junk: u32,
    #[serde(default)]
    pub good: u32,
    /// Per token hash: in how many junk and how many good messages it was.
    #[serde(default)]
    pub tokens: BTreeMap<u64, [u32; 2]>,
    /// The messages learned from (their hash) and as what, oldest first.
    #[serde(default)]
    pub seen: Vec<(u64, bool)>,
}

impl Learned {
    /// Whether it has seen enough of both kinds to file mail.
    pub fn ready(&self) -> bool {
        self.junk >= MIN_TRAINED && self.good >= MIN_TRAINED
    }

    /// Learns `m` as junk (or as good mail). Learned before as the other kind, that is taken
    /// back first; learned before as the same kind, nothing changes. `salt` is used when the
    /// filter learns its first message.
    pub fn learn(&mut self, m: &MessageHeader, junk: bool, salt: u64) {
        if self.salt == 0 {
            self.salt = salt.max(1);
        }
        let key = self.message_key(m);
        match self.seen.iter().position(|(k, _)| *k == key) {
            Some(i) if self.seen[i].1 == junk => return,
            Some(i) => {
                self.seen.remove(i);
                self.count(m, !junk, false);
            }
            None => {}
        }
        self.count(m, junk, true);
        self.seen.push((key, junk));
        if self.seen.len() > MAX_SEEN {
            self.seen.remove(0);
        }
        self.prune();
    }

    /// How much `m` looks like junk, from 0 (good) to 1 (junk); `None` before [`Self::ready`].
    pub fn score(&self, m: &MessageHeader) -> Option<f64> {
        if !self.ready() {
            return None;
        }
        let (junk, good) = (f64::from(self.junk), f64::from(self.good));
        let mut probs: Vec<f64> = self
            .hashes(m)
            .into_iter()
            .filter_map(|h| self.tokens.get(&h))
            .map(|[j, g]| {
                let (pj, pg) = (f64::from(*j) / junk, f64::from(*g) / good);
                let p = if pj + pg > 0.0 { pj / (pj + pg) } else { PRIOR };
                let n = f64::from(j + g);
                ((PRIOR_STRENGTH * PRIOR + n * p) / (PRIOR_STRENGTH + n)).clamp(0.01, 0.99)
            })
            .filter(|p| (p - 0.5).abs() > 0.1)
            .collect();
        if probs.is_empty() {
            return Some(PRIOR);
        }
        probs.sort_by(|a, b| (b - 0.5).abs().total_cmp(&(a - 0.5).abs()));
        probs.truncate(MAX_DECIDING);
        let n = probs.len();
        let s = 1.0 - chi2q(-2.0 * probs.iter().map(|p| (1.0 - p).ln()).sum::<f64>(), 2 * n);
        let h = 1.0 - chi2q(-2.0 * probs.iter().map(|p| p.ln()).sum::<f64>(), 2 * n);
        Some((s - h + 1.0) / 2.0)
    }

    /// Whether `m` goes to the Junk folder.
    pub fn is_junk(&self, m: &MessageHeader) -> bool {
        self.score(m).is_some_and(|s| s >= JUNK_SCORE)
    }

    fn count(&mut self, m: &MessageHeader, junk: bool, add: bool) {
        let slot = usize::from(!junk);
        for h in self.hashes(m) {
            let c = self.tokens.entry(h).or_insert([0, 0]);
            c[slot] = if add { c[slot].saturating_add(1) } else { c[slot].saturating_sub(1) };
            if *c == [0, 0] {
                self.tokens.remove(&h);
            }
        }
        let total = if junk { &mut self.junk } else { &mut self.good };
        *total = if add { total.saturating_add(1) } else { total.saturating_sub(1) };
    }

    /// Past [`MAX_TOKENS`], the tokens seen once go, then those seen twice, and so on.
    fn prune(&mut self) {
        let mut floor = 1;
        while self.tokens.len() > MAX_TOKENS {
            self.tokens.retain(|_, [j, g]| *j + *g > floor);
            floor += 1;
        }
    }

    fn message_key(&self, m: &MessageHeader) -> u64 {
        let id = m.message_id.as_deref().filter(|id| !id.trim().is_empty()).unwrap_or(&m.id);
        self.hash(&format!("id:{id}"))
    }

    fn hashes(&self, m: &MessageHeader) -> Vec<u64> {
        tokens(m).iter().map(|t| self.hash(t)).collect()
    }

    /// FNV-1a over the salt and the token: stable between versions, unlike std's hasher.
    fn hash(&self, token: &str) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.salt.to_le_bytes().iter().chain(token.as_bytes()) {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    }
}

/// The tokens a message is judged by, each once: the words of the subject and of the preview
/// of the text, the sender's address, domain and name, and whether it has attachments.
pub fn tokens(m: &MessageHeader) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut add = |t: String| {
        if out.len() < MAX_MESSAGE_TOKENS && seen.insert(t.clone()) {
            out.push(t);
        }
    };
    sender_tokens(&m.from).into_iter().for_each(&mut add);
    words(&m.subject).for_each(|w| add(format!("s:{w}")));
    words(&m.preview).for_each(|w| add(w.to_string()));
    if m.has_attachments {
        add("has:attachment".into());
    }
    out
}

fn sender_tokens(from: &Address) -> Vec<String> {
    let email = from.email().to_lowercase();
    let mut out = vec![format!("from:{email}")];
    if let Some((_, domain)) = email.rsplit_once('@') {
        out.push(format!("domain:{domain}"));
        // The registrable part too, so mail from a new subdomain counts.
        let parts: Vec<&str> = domain.split('.').collect();
        if parts.len() > 2 {
            out.push(format!("domain:{}", parts[parts.len() - 2..].join(".")));
        }
    }
    for w in words(from.name.as_deref().unwrap_or_default()) {
        out.push(format!("name:{w}"));
    }
    out
}

/// Lower-case words of two to thirty letters or digits.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric() && c != '€' && c != '$').filter(|w| (2..=30).contains(&w.chars().count())).map(str::to_lowercase)
}

/// The chance that chi-squared with `v` (even) degrees of freedom is at least `x2`.
fn chi2q(x2: f64, v: usize) -> f64 {
    let m = x2 / 2.0;
    let mut term = (-m).exp();
    let mut sum = term;
    for i in 1..v / 2 {
        term *= m / i as f64;
        sum += term;
    }
    sum.min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Flags, Importance, Sealed};

    fn mail(id: &str, from: &str, subject: &str, preview: &str) -> MessageHeader {
        let (name, email) = from.trim_end_matches('>').split_once(" <").unwrap();
        MessageHeader {
            id: id.into(),
            folder_id: "inbox".into(),
            remote_id: String::new(),
            from: Address::new(Some(name), email).unwrap(),
            to: Vec::new(),
            subject: subject.into(),
            date: 0,
            preview: preview.into(),
            flags: Flags::default(),
            has_attachments: false,
            message_id: Some(format!("<{id}@example.org>")),
            categories: Vec::new(),
            importance: Importance::default(),
            chat: None,
            sealed: Sealed::default(),
            size: 0,
        }
    }

    fn spam(i: usize) -> MessageHeader {
        mail(
            &format!("j{i}"),
            &format!("Winner Team <promo{i}@lucky-prize{}.biz>", i % 3),
            "You won a FREE prize",
            "Claim your free prize now, click here to win cash",
        )
    }

    fn good(i: usize) -> MessageHeader {
        let senders = ["Sanne de Vries <sanne@example.org>", "Ravi <ravi@werk.nl>", "Bank <service@bank.nl>"];
        mail(&format!("g{i}"), senders[i % 3], "Planning voor donderdag", "Zullen we het overleg over het ontwerp verplaatsen naar vrijdag?")
    }

    #[test]
    fn it_files_nothing_until_it_has_seen_both_kinds() {
        let mut l = Learned::default();
        for i in 0..20 {
            l.learn(&spam(i), true, 42);
        }
        assert!(!l.ready() && l.score(&spam(99)).is_none() && !l.is_junk(&spam(99)), "no good mail learned yet");
    }

    #[test]
    fn it_learns_junk_from_good_and_stays_unsure_about_the_unknown() {
        let mut l = Learned::default();
        for i in 0..12 {
            l.learn(&spam(i), true, 42);
            l.learn(&good(i), false, 42);
        }
        assert!(l.ready());
        let new_spam = mail("x", "Prize Desk <win@lucky-prize7.biz>", "Claim your FREE cash prize", "Click here now");
        assert!(l.score(&new_spam).unwrap() > JUNK_SCORE, "{:?}", l.score(&new_spam));
        assert!(l.is_junk(&new_spam));
        let new_good = mail("y", "Sanne de Vries <sanne@example.org>", "Overleg vrijdag", "Het ontwerp is klaar voor het overleg");
        assert!(l.score(&new_good).unwrap() < 0.1, "{:?}", l.score(&new_good));
        let unknown = mail("z", "Iemand <iemand@elders.de>", "Hallo", "Grüße aus Berlin");
        assert!(!l.is_junk(&unknown), "words never seen do not file anything");
    }

    #[test]
    fn a_change_of_mind_is_taken_back_and_learning_twice_counts_once() {
        let mut l = Learned::default();
        let m = spam(1);
        l.learn(&m, true, 7);
        l.learn(&m, true, 7);
        assert_eq!((l.junk, l.good, l.seen.len()), (1, 0, 1));
        l.learn(&m, false, 7);
        assert_eq!((l.junk, l.good, l.seen.len()), (0, 1, 1));
        assert!(l.tokens.values().all(|[j, _]| *j == 0), "its junk counts are gone");
    }

    #[test]
    fn the_file_holds_no_words() {
        let mut l = Learned::default();
        l.learn(&spam(1), true, 1234);
        let json = serde_json::to_string(&l).unwrap();
        assert!(!json.contains("prize") && !json.contains("lucky") && !json.contains("promo"), "{json}");
        let mut other = Learned::default();
        other.learn(&spam(1), true, 5678);
        assert_ne!(l.tokens.keys().next(), other.tokens.keys().next(), "another installation, other hashes");
    }

    #[test]
    fn the_tokens_cover_sender_subject_and_text_once() {
        let t = tokens(&mail("a", "Promo <deals@mail.shop.example>", "Deals deals", "Big deals today"));
        assert!(t.contains(&"from:deals@mail.shop.example".to_string()));
        assert!(t.contains(&"domain:mail.shop.example".to_string()) && t.contains(&"domain:shop.example".to_string()));
        assert!(t.contains(&"name:promo".to_string()) && t.contains(&"s:deals".to_string()) && t.contains(&"deals".to_string()));
        assert_eq!(t.iter().filter(|w| *w == "s:deals").count(), 1);
    }
}
