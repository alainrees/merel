//! The embedding model: a BERT encoder with the e5 recipe (a "query: " or "passage: " prefix,
//! the mean of the last layer, scaled to length one).

use std::collections::HashMap;
use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::{Tokenizer, TruncationParams};

use crate::{Error, Result};

/// The length of a vector.
pub const DIM: usize = 384;
/// The most tokens of a text the model reads; the start of a message says what it is about.
pub const MAX_TOKENS: usize = 256;
/// The files [`crate::setup`] leaves, which [`Embedder::load`] reads.
pub const MODEL_FILE: &str = "model-nl-en.safetensors";
pub const TOKENIZER_FILE: &str = "tokenizer-nl-en.json";

/// What a text is: the words of a search, or a message to be found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Query,
    Passage,
}

/// multilingual-e5-small's shape, with the number of words the kept vocabulary has.
fn config(vocab: usize, layers: usize, hidden: usize, heads: usize, intermediate: usize) -> Result<Config> {
    let json = format!(
        r#"{{"vocab_size":{vocab},"hidden_size":{hidden},"num_hidden_layers":{layers},"num_attention_heads":{heads},
        "intermediate_size":{intermediate},"hidden_act":"gelu","hidden_dropout_prob":0.0,"max_position_embeddings":512,
        "type_vocab_size":2,"initializer_range":0.02,"layer_norm_eps":1e-12,"pad_token_id":0,
        "position_embedding_type":"absolute","use_cache":false,"classifier_dropout":null,"model_type":"bert"}}"#
    );
    serde_json::from_str(&json).map_err(|e| Error::Model(e.to_string()))
}

/// The name of the word embeddings in a set of tensors, with or without a `bert.` in front.
pub(crate) fn word_embeddings(names: impl Iterator<Item = String>) -> Option<String> {
    names.into_iter().find(|n| n.ends_with("embeddings.word_embeddings.weight"))
}

pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl Embedder {
    /// The model [`crate::setup`] prepared in `dir`.
    pub fn load(dir: &Path) -> Result<Embedder> {
        let weights = std::fs::read(dir.join(MODEL_FILE))?;
        let tokenizer = Tokenizer::from_file(dir.join(TOKENIZER_FILE)).map_err(|e| Error::Model(e.to_string()))?;
        Embedder::from_parts(&weights, tokenizer)
    }

    /// A model from its weights (safetensors) and its tokenizer. The shape is read from the
    /// weights, so a small model for tests works the same way.
    pub fn from_parts(weights: &[u8], tokenizer: Tokenizer) -> Result<Embedder> {
        let device = Device::Cpu;
        let tensors: HashMap<String, Tensor> = candle_core::safetensors::load_buffer(weights, &device)?;
        let name = word_embeddings(tensors.keys().cloned()).ok_or_else(|| Error::Model("no word embeddings".into()))?;
        let (vocab, hidden) = tensors[&name].dims2()?;
        let prefix = name.trim_end_matches("embeddings.word_embeddings.weight");
        let layers = (0..).take_while(|i| tensors.contains_key(&format!("{prefix}encoder.layer.{i}.attention.self.query.weight"))).count();
        let intermediate = tensors
            .get(&format!("{prefix}encoder.layer.0.intermediate.dense.weight"))
            .map(|t| t.dims2().map(|(rows, _)| rows))
            .transpose()?
            .ok_or_else(|| Error::Model("no encoder layers".into()))?;
        // multilingual-e5-small has heads of 32 dimensions.
        let heads = (hidden / 32).max(1);
        let config = config(vocab, layers, hidden, heads, intermediate)?;
        let vb = VarBuilder::from_tensors(tensors, DType::F32, &device);
        let model = BertModel::load(vb, &config)?;
        let mut tokenizer = tokenizer;
        tokenizer
            .with_truncation(Some(TruncationParams { max_length: MAX_TOKENS, ..Default::default() }))
            .map_err(|e| Error::Model(e.to_string()))?;
        tokenizer.with_padding(None);
        Ok(Embedder { model, tokenizer, device })
    }

    /// The vector of a text, of length one.
    pub fn embed(&self, text: &str, kind: Kind) -> Result<Vec<f32>> {
        let prefix = match kind {
            Kind::Query => "query: ",
            Kind::Passage => "passage: ",
        };
        let encoding = self.tokenizer.encode(format!("{prefix}{text}"), true).map_err(|e| Error::Model(e.to_string()))?;
        let ids = encoding.get_ids();
        if ids.is_empty() {
            return Err(Error::Model("nothing to read".into()));
        }
        let input = Tensor::new(ids, &self.device)?.unsqueeze(0)?;
        let types = input.zeros_like()?;
        let out = self.model.forward(&input, &types, None)?;
        let mean = out.mean(1)?.squeeze(0)?;
        let v: Vec<f32> = mean.to_vec1()?;
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if !norm.is_finite() || norm == 0.0 {
            return Err(Error::Model("an empty vector".into()));
        }
        Ok(v.into_iter().map(|x| x / norm).collect())
    }
}

/// A tiny model with random weights and a tokenizer of a few words, the same shape as the
/// real one, for tests of everything around it; `None` only when the temporary directory
/// cannot be written.
#[doc(hidden)]
pub fn test_model() -> Option<(Vec<u8>, String)> {
    let device = Device::Cpu;
    let words = ["<s>", "<pad>", "</s>", "<unk>", "▁query", ":", "▁passage", "▁keuken", "▁aannemer", "▁factuur", "▁кухня", "▁"];
    let vocab = words.len();
    let hidden = 64;
    let mut tensors: HashMap<String, Tensor> = HashMap::new();
    let mut add = |name: &str, dims: &[usize]| {
        let t = if name.ends_with("LayerNorm.weight") { Tensor::ones(dims, DType::F32, &device) } else { Tensor::randn(0f32, 0.2, dims, &device) };
        if let Ok(t) = t {
            tensors.insert(name.to_string(), t);
        }
    };
    add("embeddings.word_embeddings.weight", &[vocab, hidden]);
    add("embeddings.position_embeddings.weight", &[512, hidden]);
    add("embeddings.token_type_embeddings.weight", &[2, hidden]);
    add("embeddings.LayerNorm.weight", &[hidden]);
    add("embeddings.LayerNorm.bias", &[hidden]);
    for l in 0..2 {
        let p = format!("encoder.layer.{l}.");
        for m in ["attention.self.query", "attention.self.key", "attention.self.value", "attention.output.dense"] {
            add(&format!("{p}{m}.weight"), &[hidden, hidden]);
            add(&format!("{p}{m}.bias"), &[hidden]);
        }
        add(&format!("{p}attention.output.LayerNorm.weight"), &[hidden]);
        add(&format!("{p}attention.output.LayerNorm.bias"), &[hidden]);
        add(&format!("{p}intermediate.dense.weight"), &[128, hidden]);
        add(&format!("{p}intermediate.dense.bias"), &[128]);
        add(&format!("{p}output.dense.weight"), &[hidden, 128]);
        add(&format!("{p}output.dense.bias"), &[hidden]);
        add(&format!("{p}output.LayerNorm.weight"), &[hidden]);
        add(&format!("{p}output.LayerNorm.bias"), &[hidden]);
    }
    // Tests run side by side: each gets a file of its own.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("merel-tiny-{}-{n}.safetensors", std::process::id()));
    candle_core::safetensors::save(&tensors, &path).ok()?;
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let vocab_json: Vec<String> = words.iter().map(|w| format!("[\"{w}\",-{}.0]", if w.starts_with('<') { 0 } else { 5 })).collect();
    let tokenizer = format!(
        r#"{{"version":"1.0","truncation":null,"padding":null,
        "added_tokens":[{{"id":0,"content":"<s>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},
        {{"id":1,"content":"<pad>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},
        {{"id":2,"content":"</s>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},
        {{"id":3,"content":"<unk>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}}],
        "normalizer":null,
        "pre_tokenizer":{{"type":"Metaspace","replacement":"▁","prepend_scheme":"always","split":true}},
        "post_processor":{{"type":"TemplateProcessing","single":[{{"SpecialToken":{{"id":"<s>","type_id":0}}}},{{"Sequence":{{"id":"A","type_id":0}}}},{{"SpecialToken":{{"id":"</s>","type_id":0}}}}],
          "pair":[{{"Sequence":{{"id":"A","type_id":0}}}},{{"Sequence":{{"id":"B","type_id":1}}}}],
          "special_tokens":{{"<s>":{{"id":"<s>","ids":[0],"tokens":["<s>"]}},"</s>":{{"id":"</s>","ids":[2],"tokens":["</s>"]}}}}}},
        "decoder":{{"type":"Metaspace","replacement":"▁","prepend_scheme":"always","split":true}},
        "model":{{"type":"Unigram","unk_id":3,"vocab":[{}],"byte_fallback":false}}}}"#,
        vocab_json.join(",")
    );
    Some((bytes, tokenizer))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn tiny() -> (Vec<u8>, String) {
        test_model().unwrap()
    }

    pub fn tiny_embedder() -> Embedder {
        let (weights, tokenizer) = tiny();
        Embedder::from_parts(&weights, Tokenizer::from_bytes(tokenizer.as_bytes()).unwrap()).unwrap()
    }

    #[test]
    fn a_text_becomes_a_vector_of_length_one_and_the_same_text_the_same_vector() {
        let e = tiny_embedder();
        let a = e.embed("keuken aannemer", Kind::Passage).unwrap();
        assert_eq!(a.len(), 64);
        let norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "{norm}");
        assert_eq!(a, e.embed("keuken aannemer", Kind::Passage).unwrap());
        assert_ne!(a, e.embed("keuken aannemer", Kind::Query).unwrap(), "a search and a message get their own prefix");
        let long = "keuken ".repeat(2000);
        assert!(e.embed(&long, Kind::Passage).is_ok(), "a long text is cut to what the model reads");
    }
}
