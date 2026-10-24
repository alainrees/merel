//! Getting the model onto this computer: downloaded once from a fixed revision, every file
//! checked against its SHA-256, then cut down to what Dutch and English mail can use and kept
//! at half precision. Nothing is used before it passed the check.

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use candle_core::{DType, Device, Tensor};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::model::{MODEL_FILE, TOKENIZER_FILE, word_embeddings};
use crate::{Error, Result};

/// The model's repository and the revision whose files are checked below.
const REPOSITORY: &str = "intfloat/multilingual-e5-small";
const REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";

/// A file to download: its name, size and SHA-256.
pub struct Source {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const SOURCES: [Source; 2] = [
    Source { name: "model.safetensors", size: 470_641_600, sha256: "1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477" },
    Source { name: "tokenizer.json", size: 17_082_730, sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39" },
];

/// Megabytes the download takes, for the settings page.
pub fn download_megabytes() -> u64 {
    SOURCES.iter().map(|s| s.size).sum::<u64>() / 1_000_000
}

/// Says which version of the preparation made the files in a directory.
const READY_FILE: &str = "ready";
const READY: &str = "multilingual-e5-small 614241f nl-en f16 1";

/// How far getting the model is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Downloading { done: u64, total: u64 },
    Preparing,
}

pub fn url(name: &str) -> String {
    format!("https://huggingface.co/{REPOSITORY}/resolve/{REVISION}/{name}")
}

/// Whether `dir` holds a prepared model.
pub fn is_ready(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(READY_FILE)).is_ok_and(|s| s.trim() == READY)
        && dir.join(MODEL_FILE).is_file()
        && dir.join(TOKENIZER_FILE).is_file()
}

/// Bytes the prepared model takes on disk.
pub fn size_on_disk(dir: &Path) -> u64 {
    [MODEL_FILE, TOKENIZER_FILE].iter().filter_map(|f| std::fs::metadata(dir.join(f)).ok()).map(|m| m.len()).sum()
}

/// Throws the model away.
pub fn remove(dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

fn tls() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .build()
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .tls_config(tls())
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(60 * 60)))
            .https_only(true)
            // Hugging Face serves large files from its storage host after a redirect.
            .max_redirects(5)
            .user_agent("Merel")
            .build(),
    )
}

/// Downloads, checks and prepares the model in `dir`. A file that came in whole before is not
/// fetched again.
pub fn install(dir: &Path, cancel: &AtomicBool, report: &dyn Fn(Stage)) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let agent = agent();
    let total: u64 = SOURCES.iter().map(|s| s.size).sum();
    let mut before = 0;
    for source in &SOURCES {
        let path = dir.join(source.name);
        if !(path.is_file() && sha256_of(&path)? == source.sha256) {
            download(&agent, source, dir, cancel, &|done| report(Stage::Downloading { done: before + done, total }))?;
        }
        before += source.size;
    }
    report(Stage::Preparing);
    let weights = std::fs::read(dir.join(SOURCES[0].name))?;
    let tokenizer = std::fs::read_to_string(dir.join(SOURCES[1].name))?;
    let (weights, tokenizer) = compact(&weights, &tokenizer)?;
    write_atomic(&dir.join(MODEL_FILE), &weights)?;
    write_atomic(&dir.join(TOKENIZER_FILE), tokenizer.as_bytes())?;
    for source in &SOURCES {
        let _ = std::fs::remove_file(dir.join(source.name));
    }
    write_atomic(&dir.join(READY_FILE), READY.as_bytes())?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let part = path.with_extension("part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)?;
    Ok(())
}

fn sha256_of(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn download(agent: &ureq::Agent, source: &Source, dir: &Path, cancel: &AtomicBool, report: &dyn Fn(u64)) -> Result<()> {
    let failed = |e: &dyn std::fmt::Display| Error::Download(e.to_string());
    let mut response = agent.get(&url(source.name)).call().map_err(|e| failed(&e))?;
    let mut body = response.body_mut().with_config().limit(source.size + 1).reader();
    let part = dir.join(format!("{}.part", source.name));
    let mut out = std::fs::File::create(&part)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err(Error::Cancelled);
        }
        let n = body.read(&mut buffer).map_err(|e| failed(&e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        out.write_all(&buffer[..n])?;
        done += n as u64;
        report(done);
    }
    out.sync_all()?;
    drop(out);
    if done != source.size || hex(&hasher.finalize()) != source.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(Error::Mismatch(source.name.to_string()));
    }
    std::fs::rename(&part, dir.join(source.name))?;
    Ok(())
}

/// Whether Dutch and English text can ever be cut into this piece of the vocabulary: Latin
/// letters, digits, punctuation, currency signs and the word-start mark.
fn latin(piece: &str) -> bool {
    piece.chars().all(|c| matches!(c as u32, 0x20..=0x7E | 0xA0..=0x24F | 0x2000..=0x206F | 0x20A0..=0x20CF | 0x2581))
}

/// The model and tokenizer cut down to the pieces of the vocabulary that Latin script can
/// produce, with every id renumbered to match, and the weights at half precision. Text in
/// Latin script is cut into exactly the same pieces as before, since the pieces taken away
/// hold characters it does not have.
pub fn compact(weights: &[u8], tokenizer: &str) -> Result<(Vec<u8>, String)> {
    let bad = |what: &str| Error::Model(format!("unexpected tokenizer: {what}"));
    let mut json: Value = serde_json::from_str(tokenizer).map_err(|e| Error::Model(e.to_string()))?;
    let vocab = json.pointer("/model/vocab").and_then(Value::as_array).ok_or_else(|| bad("no Unigram vocabulary"))?;
    let mut keep: BTreeSet<usize> = (0..4.min(vocab.len())).collect();
    if let Some(added) = json.get("added_tokens").and_then(Value::as_array) {
        keep.extend(added.iter().filter_map(|t| t.get("id").and_then(Value::as_u64)).map(|i| i as usize).filter(|i| *i < vocab.len()));
    }
    for (i, entry) in vocab.iter().enumerate() {
        if entry.get(0).and_then(Value::as_str).is_some_and(latin) {
            keep.insert(i);
        }
    }
    let new_id: HashMap<usize, usize> = keep.iter().enumerate().map(|(new, old)| (*old, new)).collect();
    let map = |v: &mut Value| -> Result<()> {
        let old = v.as_u64().ok_or_else(|| bad("an id that is no number"))? as usize;
        *v = Value::from(*new_id.get(&old).ok_or_else(|| bad("an id outside the vocabulary"))? as u64);
        Ok(())
    };
    let kept: Vec<Value> = keep.iter().map(|i| vocab[*i].clone()).collect();
    if let Some(v) = json.pointer_mut("/model/vocab") {
        *v = Value::Array(kept);
    }
    if let Some(unk) = json.pointer_mut("/model/unk_id").filter(|v| !v.is_null()) {
        map(unk)?;
    }
    if let Some(added) = json.get_mut("added_tokens").and_then(Value::as_array_mut) {
        added.retain(|t| t.get("id").and_then(Value::as_u64).is_some_and(|i| new_id.contains_key(&(i as usize))));
        for t in added.iter_mut() {
            if let Some(id) = t.get_mut("id") {
                map(id)?;
            }
        }
    }
    if let Some(post) = json.get_mut("post_processor").filter(|p| !p.is_null()) {
        match post.get("type").and_then(Value::as_str) {
            Some("TemplateProcessing") => {
                if let Some(specials) = post.get_mut("special_tokens").and_then(Value::as_object_mut) {
                    for special in specials.values_mut() {
                        if let Some(ids) = special.get_mut("ids").and_then(Value::as_array_mut) {
                            for id in ids.iter_mut() {
                                map(id)?;
                            }
                        }
                    }
                }
            }
            Some("RobertaProcessing" | "BertProcessing") => {
                for key in ["sep", "cls"] {
                    if let Some(id) = post.get_mut(key).and_then(|v| v.get_mut(1)) {
                        map(id)?;
                    }
                }
            }
            other => return Err(bad(&format!("post processor {other:?}"))),
        }
    }
    if let Some(pad) = json.pointer_mut("/padding/pad_id") {
        map(pad)?;
    }
    let tokenizer = serde_json::to_string(&json).map_err(|e| Error::Model(e.to_string()))?;

    let device = Device::Cpu;
    let mut tensors: HashMap<String, Tensor> = candle_core::safetensors::load_buffer(weights, &device)?;
    let name = word_embeddings(tensors.keys().cloned()).ok_or_else(|| Error::Model("no word embeddings".into()))?;
    let rows = tensors[&name].dims2()?.0;
    if keep.last().is_some_and(|last| *last >= rows) {
        return Err(Error::Model("the tokenizer has more words than the model".into()));
    }
    let index: Vec<u32> = keep.iter().map(|i| *i as u32).collect();
    let index = Tensor::new(index.as_slice(), &device)?;
    let embeddings = tensors[&name].index_select(&index, 0)?;
    tensors.insert(name, embeddings);
    let half: HashMap<String, Tensor> =
        tensors.into_iter().map(|(n, t)| t.to_dtype(DType::F16).map(|t| (n, t))).collect::<std::result::Result<_, _>>()?;
    let path = std::env::temp_dir().join(format!("merel-compact-{}.safetensors", std::process::id()));
    candle_core::safetensors::save(&half, &path)?;
    let bytes = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    Ok((bytes?, tokenizer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;
    use crate::model::tests::tiny;
    use tokenizers::Tokenizer;

    #[test]
    fn compacting_keeps_latin_words_renumbers_them_and_halves_the_weights() {
        let (weights, tokenizer) = tiny();
        let before = Tokenizer::from_bytes(tokenizer.as_bytes()).unwrap();
        let (small, small_tokenizer) = compact(&weights, &tokenizer).unwrap();
        let after = Tokenizer::from_bytes(small_tokenizer.as_bytes()).unwrap();
        assert_eq!(before.get_vocab_size(true), 12);
        assert_eq!(after.get_vocab_size(true), 11, "the Cyrillic word is gone");
        assert!(small.len() < weights.len() * 6 / 10, "{} of {}", small.len(), weights.len());
        let tokens = |t: &Tokenizer| t.encode("keuken factuur", true).unwrap().get_tokens().to_vec();
        assert_eq!(tokens(&before), tokens(&after), "Latin text is cut the same way");
        let ids = after.encode("keuken", true).unwrap().get_ids().to_vec();
        assert_eq!(ids.first(), Some(&0), "<s> keeps its place");
        assert_eq!(ids.last(), Some(&2), "and </s>");

        // The two models give the same vector for Latin text, up to half precision.
        let full = crate::Embedder::from_parts(&weights, before).unwrap();
        let cut = crate::Embedder::from_parts(&small, after).unwrap();
        let a = full.embed("keuken aannemer", Kind::Passage).unwrap();
        let b = cut.embed("keuken aannemer", Kind::Passage).unwrap();
        let cos: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!(cos > 0.999, "{cos}");
    }

    #[test]
    fn latin_pieces_are_the_ones_dutch_and_english_text_can_make() {
        for piece in ["▁keuken", "é", "▁€", "ß", "“", "12,50", "▁"] {
            assert!(latin(piece), "{piece}");
        }
        for piece in ["▁кухня", "日本", "▁مرحبا", "😀"] {
            assert!(!latin(piece), "{piece}");
        }
    }

    #[test]
    fn a_ready_directory_is_recognised_and_removed() {
        let dir = std::env::temp_dir().join(format!("merel-semantic-ready-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_ready(&dir));
        let (weights, tokenizer) = tiny();
        let (w, t) = compact(&weights, &tokenizer).unwrap();
        std::fs::write(dir.join(MODEL_FILE), w).unwrap();
        std::fs::write(dir.join(TOKENIZER_FILE), t).unwrap();
        assert!(!is_ready(&dir), "not without the mark of a finished preparation");
        std::fs::write(dir.join(READY_FILE), READY).unwrap();
        assert!(is_ready(&dir));
        assert!(crate::Embedder::load(&dir).is_ok());
        assert!(size_on_disk(&dir) > 0);
        remove(&dir).unwrap();
        assert!(!dir.exists());
        assert_eq!(url("tokenizer.json"), format!("https://huggingface.co/{REPOSITORY}/resolve/{REVISION}/tokenizer.json"));
    }
}

/// A prepared tiny model in a directory, for tests elsewhere in Merel.
#[doc(hidden)]
pub mod tests_support {
    use super::*;

    pub fn prepare(dir: &Path) {
        let Some((weights, tokenizer)) = crate::model::test_model() else { return };
        if let (Ok(()), Ok((w, t))) = (std::fs::create_dir_all(dir), compact(&weights, &tokenizer)) {
            let _ = std::fs::write(dir.join(MODEL_FILE), w);
            let _ = std::fs::write(dir.join(TOKENIZER_FILE), t);
            let _ = std::fs::write(dir.join(READY_FILE), READY);
        }
    }
}
