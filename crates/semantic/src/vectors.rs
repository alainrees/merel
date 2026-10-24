//! The vectors of the mail: one per message, kept as bytes (each number scaled to -127..127)
//! so a hundred thousand messages take under 40 MB, and compared one by one, which for that
//! many takes milliseconds. The engine seals them on disk like the index.

use std::collections::HashMap;

/// Starts the file, with a version.
const MAGIC: &[u8; 4] = b"MVE1";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vectors {
    dim: usize,
    map: HashMap<String, (f32, Vec<i8>)>,
}

fn quantize(v: &[f32]) -> (f32, Vec<i8>) {
    let max = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    let scale = if max > 0.0 { max / 127.0 } else { 1.0 };
    (scale, v.iter().map(|x| (x / scale).round().clamp(-127.0, 127.0) as i8).collect())
}

impl Vectors {
    pub fn new(dim: usize) -> Vectors {
        Vectors { dim, map: HashMap::new() }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }

    /// Keeps a message's vector; one of another length is ignored.
    pub fn insert(&mut self, id: &str, v: &[f32]) {
        if v.len() == self.dim {
            self.map.insert(id.to_string(), quantize(v));
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.map.remove(id).is_some()
    }

    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.map.retain(|id, _| keep(id));
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// The `k` messages whose vectors lie closest to `query` (cosine similarity), among those
    /// `allowed` lets through, best first.
    pub fn nearest(&self, query: &[f32], k: usize, allowed: impl Fn(&str) -> bool) -> Vec<(String, f32)> {
        if query.len() != self.dim || k == 0 {
            return Vec::new();
        }
        let mut best: Vec<(&str, f32)> = Vec::with_capacity(k + 1);
        for (id, (scale, v)) in &self.map {
            let dot: f32 = v.iter().zip(query).map(|(a, b)| f32::from(*a) * b).sum::<f32>() * scale;
            if best.len() == k && best.last().is_some_and(|(_, s)| *s >= dot) {
                continue;
            }
            if !allowed(id) {
                continue;
            }
            let at = best.partition_point(|(_, s)| *s >= dot);
            best.insert(at, (id, dot));
            best.truncate(k);
        }
        best.into_iter().map(|(id, s)| (id.to_string(), s)).collect()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.map.len() * (self.dim + 40));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.dim as u32).to_le_bytes());
        out.extend_from_slice(&(self.map.len() as u32).to_le_bytes());
        for (id, (scale, v)) in &self.map {
            out.extend_from_slice(&(id.len() as u16).to_le_bytes());
            out.extend_from_slice(id.as_bytes());
            out.extend_from_slice(&scale.to_le_bytes());
            out.extend(v.iter().map(|x| *x as u8));
        }
        out
    }

    /// The vectors from [`Vectors::to_bytes`]; `None` for anything else.
    pub fn from_bytes(bytes: &[u8]) -> Option<Vectors> {
        let mut at = 0;
        let mut take = |n: usize| -> Option<&[u8]> {
            let part = bytes.get(at..at + n)?;
            at += n;
            Some(part)
        };
        if take(4)? != MAGIC {
            return None;
        }
        let dim = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let count = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let mut map = HashMap::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let len = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
            let id = std::str::from_utf8(take(len)?).ok()?.to_string();
            let scale = f32::from_le_bytes(take(4)?.try_into().ok()?);
            let v = take(dim)?.iter().map(|b| *b as i8).collect();
            map.insert(id, (scale, v));
        }
        Some(Vectors { dim, map })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    #[test]
    fn the_nearest_come_first_within_what_is_allowed_and_survive_a_round_trip() {
        let mut vs = Vectors::new(3);
        vs.insert("keuken", &unit(&[1.0, 0.1, 0.0]));
        vs.insert("aannemer", &unit(&[0.9, 0.3, 0.0]));
        vs.insert("factuur", &unit(&[0.0, 0.0, 1.0]));
        vs.insert("kort", &[1.0]);
        assert_eq!(vs.len(), 3, "a vector of another length is ignored");
        let q = unit(&[1.0, 0.0, 0.0]);
        let ids: Vec<String> = vs.nearest(&q, 2, |_| true).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["keuken", "aannemer"]);
        let ids: Vec<String> = vs.nearest(&q, 2, |id| id != "keuken").into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["aannemer", "factuur"]);
        let score = vs.nearest(&q, 1, |_| true)[0].1;
        assert!((score - unit(&[1.0, 0.1, 0.0])[0]).abs() < 0.02, "{score}");
        let back = Vectors::from_bytes(&vs.to_bytes()).unwrap();
        assert_eq!(back, vs);
        assert!(Vectors::from_bytes(b"MVE1\x03\0\0\0\x05\0\0\0").is_none(), "a cut-off file");
        assert!(Vectors::from_bytes(b"nope").is_none());
        vs.retain(|id| id != "factuur");
        assert!(!vs.contains("factuur") && vs.remove("keuken") && !vs.remove("keuken"));
    }
}
