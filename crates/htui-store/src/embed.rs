//! Dense embeddings for the concepts index (MOD-34, `docs/ANA-20.md` §3.1).
//!
//! [`DenseEmbedder`] is the seam the vector store embeds through. Production uses
//! [`FastEmbedder`] (local ONNX via `fastembed-rs`, behind the `local-embed` feature because
//! `ort` downloads onnxruntime at build time); tests use [`HashEmbedder`], which needs neither a
//! model nor a network. The sparse half of hybrid search is not a model at all: see
//! [`crate::bm25`].
use htui_core::store::StoreError;
#[cfg(feature = "local-embed")]
use std::{fmt, sync::Arc};

#[cfg(feature = "local-embed")]
use crate::model::{self, ModelFiles};

/// Width of BGE-small-en-v1.5's vectors, and so of the index's `dense` vector.
pub const DENSE_DIM: usize = 384;

/// Turns texts into dense vectors of a fixed width.
#[allow(async_fn_in_trait)]
pub trait DenseEmbedder {
    /// Width of every vector [`embed`](DenseEmbedder::embed) returns.
    fn dim(&self) -> usize;

    /// One vector per text, in input order.
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, StoreError>;
}

/// Most padded tokens (rows × longest row) one forward pass may hold: 16 rows of 512 (MOD-68 D5).
#[cfg(any(test, feature = "local-embed"))]
pub(crate) const MAX_BATCH_TOKENS: usize = 16 * 512;

/// Splits texts of token lengths `lens`, in input order, into consecutive ranges whose
/// `rows × longest` stays within `cap`. A single text longer than `cap` is a range of its own.
#[cfg(any(test, feature = "local-embed"))]
pub(crate) fn sub_batches(lens: &[usize], cap: usize) -> Vec<std::ops::Range<usize>> {
    let cap = cap.max(1);
    let mut ranges = Vec::new();
    let (mut start, mut longest) = (0, 0);
    for (i, &len) in lens.iter().enumerate() {
        let widest = longest.max(len);
        if i > start && (i - start + 1) * widest > cap {
            ranges.push(start..i);
            start = i;
            longest = len;
        } else {
            longest = widest;
        }
    }
    if start < lens.len() {
        ranges.push(start..lens.len());
    }
    ranges
}

/// BGE-small-en-v1.5 on `rten`, run on the blocking pool (MOD-68 D3). Cheap to clone: the model,
/// the tokenizer and the thread pool are shared (MOD-64 D238).
#[cfg(feature = "local-embed")]
#[derive(Clone)]
pub struct RtenEmbedder {
    inner: Arc<Rten>,
}

/// What one loaded model holds: the graph, its tokenizer, a thread pool and the graph's node ids.
#[cfg(feature = "local-embed")]
struct Rten {
    model: rten::Model,
    tokenizer: tokenizers::Tokenizer,
    pool: Arc<rten::ThreadPool>,
    input_ids: rten::NodeId,
    attention_mask: rten::NodeId,
    token_type_ids: rten::NodeId,
    output: rten::NodeId,
}

#[cfg(feature = "local-embed")]
impl fmt::Debug for RtenEmbedder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtenEmbedder").finish_non_exhaustive()
    }
}

#[cfg(feature = "local-embed")]
impl RtenEmbedder {
    /// Loads the model and tokenizer from `files`. CPU only; never touches the network.
    ///
    /// # Errors
    /// [`StoreError::Backend`] naming the file that would not load.
    pub fn load(files: &ModelFiles) -> Result<Self, StoreError> {
        use tokenizers::{AddedToken, Tokenizer, TruncationParams};

        let cannot = |path: &std::path::Path, e: &dyn fmt::Display| {
            StoreError::Backend(format!(
                "embedding model: cannot load {}: {e}",
                path.display()
            ))
        };
        // The tokenizer first: it is cheap, and it fails fast on a wrong directory.
        let mut tokenizer =
            Tokenizer::from_file(&files.tokenizer).map_err(|e| cannot(&files.tokenizer, &e))?;
        // Truncation only: padding is per sub-batch, by hand, in `embed_blocking` (MOD-68 A-2).
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: model::MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(|e| cannot(&files.tokenizer, &e))?;
        // fastembed's own call, from constants: it re-added `special_tokens_map.json`'s tokens.
        tokenizer
            .add_special_tokens(model::SPECIAL_TOKENS.map(|t| AddedToken {
                content: t.into(),
                special: true,
                ..Default::default()
            }))
            .map_err(|e| cannot(&files.tokenizer, &e))?;

        let graph = rten::Model::load_file(&files.onnx).map_err(|e| cannot(&files.onnx, &e))?;
        let node = |name: &str| graph.node_id(name).map_err(|e| cannot(&files.onnx, &e));
        let input_ids = node("input_ids")?;
        let attention_mask = node("attention_mask")?;
        let token_type_ids = node("token_type_ids")?;
        let output = *graph.output_ids().first().ok_or_else(|| {
            StoreError::Backend(format!(
                "embedding model: {} has no output",
                files.onnx.display()
            ))
        })?;
        let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        Ok(Self {
            inner: Arc::new(Rten {
                model: graph,
                tokenizer,
                pool: Arc::new(rten::ThreadPool::with_num_threads(threads)),
                input_ids,
                attention_mask,
                token_type_ids,
                output,
            }),
        })
    }

    /// One vector per text, in input order, at most `cap` padded tokens per forward pass.
    fn embed_blocking(&self, texts: &[String], cap: usize) -> Result<Vec<Vec<f32>>, StoreError> {
        use rten_tensor::NdTensor;
        use rten_tensor::prelude::*;

        let failed = |e: &dyn fmt::Display| StoreError::Backend(format!("embedding failed: {e}"));
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let rten = &*self.inner;
        let encodings = rten
            .tokenizer
            .encode_batch(texts.iter().map(String::as_str).collect::<Vec<_>>(), true)
            .map_err(|e| failed(&e))?;
        let lens: Vec<usize> = encodings.iter().map(|e| e.get_ids().len()).collect();

        let mut vectors = Vec::with_capacity(texts.len());
        for range in sub_batches(&lens, cap) {
            let rows = range.len();
            let longest = lens[range.clone()].iter().copied().max().unwrap_or(0);
            // Right-side padding to this sub-batch's longest row: `[PAD]` (`model::PAD_ID`, 0),
            // mask 0, type 0, exactly what `PaddingStrategy::BatchLongest` makes of it.
            let mut ids = vec![model::PAD_ID as i32; rows * longest];
            let mut mask = vec![0_i32; rows * longest];
            let mut types = vec![0_i32; rows * longest];
            for (row, encoding) in encodings[range].iter().enumerate() {
                let at = row * longest;
                for (k, ((&id, &m), &t)) in encoding
                    .get_ids()
                    .iter()
                    .zip(encoding.get_attention_mask())
                    .zip(encoding.get_type_ids())
                    .enumerate()
                {
                    ids[at + k] = id as i32;
                    mask[at + k] = m as i32;
                    types[at + k] = t as i32;
                }
            }
            let ids = NdTensor::from_data([rows, longest], ids);
            let mask = NdTensor::from_data([rows, longest], mask);
            let types = NdTensor::from_data([rows, longest], types);

            let mut opts = rten::RunOptions::default();
            opts.thread_pool = Some(Arc::clone(&rten.pool));
            let mut outputs = rten
                .model
                .run(
                    vec![
                        (rten.input_ids, ids.view().into()),
                        (rten.attention_mask, mask.view().into()),
                        (rten.token_type_ids, types.view().into()),
                    ],
                    &[rten.output],
                    Some(opts),
                )
                .map_err(|e| failed(&e))?;
            let hidden: NdTensor<f32, 3> = outputs
                .pop()
                .ok_or_else(|| failed(&"the model answered nothing"))?
                .try_into()
                .map_err(|e| failed(&e))?;
            let width = hidden.shape()[2];
            if width != DENSE_DIM {
                return Err(failed(&format_args!(
                    "the model answered {width}-wide vectors, expected {DENSE_DIM}"
                )));
            }
            for row in 0..rows {
                // The `[CLS]` token's hidden state, then fastembed's normalisation exactly.
                let v: Vec<f32> = hidden.slice((row, 0)).iter().copied().collect();
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                vectors.push(v.iter().map(|x| x / (norm + 1e-12)).collect());
            }
        }
        Ok(vectors)
    }
}

#[cfg(feature = "local-embed")]
impl DenseEmbedder for RtenEmbedder {
    fn dim(&self) -> usize {
        DENSE_DIM
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, StoreError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.embed_blocking(&texts, MAX_BATCH_TOKENS))
            .await
            .map_err(|e| StoreError::Backend(format!("embedding task failed: {e}")))?
    }
}

/// BGE-small-en-v1.5 through `fastembed-rs`, run on the blocking pool. Cheap to clone: the
/// model is shared (MOD-64 D238).
#[cfg(feature = "local-embed")]
#[derive(Clone)]
pub struct FastEmbedder {
    model: Arc<fastembed::TextEmbedding>,
}

#[cfg(feature = "local-embed")]
impl fmt::Debug for FastEmbedder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FastEmbedder").finish_non_exhaustive()
    }
}

#[cfg(feature = "local-embed")]
impl FastEmbedder {
    /// Loads the model, downloading it into the user's cache directory on first use
    /// (`<cache>/htui/fastembed`, not fastembed's default of a directory under the working one).
    pub fn new() -> Result<Self, StoreError> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

        let mut options = InitOptions {
            model_name: EmbeddingModel::BGESmallENV15,
            show_download_progress: false,
            ..Default::default()
        };
        if let Some(cache) = dirs::cache_dir() {
            options.cache_dir = cache.join("htui").join("fastembed");
        }
        let model = TextEmbedding::try_new(options)
            .map_err(|e| StoreError::Backend(format!("failed to load the embedding model: {e}")))?;
        Ok(Self {
            model: Arc::new(model),
        })
    }
}

#[cfg(feature = "local-embed")]
impl DenseEmbedder for FastEmbedder {
    fn dim(&self) -> usize {
        DENSE_DIM
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, StoreError> {
        let model = self.model.clone();
        tokio::task::spawn_blocking(move || {
            model
                .embed(texts, None)
                .map_err(|e| StoreError::Backend(format!("embedding failed: {e}")))
        })
        .await
        .map_err(|e| StoreError::Backend(format!("embedding task failed: {e}")))?
    }
}

/// A deterministic stand-in for a model: each lowercase alphanumeric word adds 1 to the dimension
/// its SHA-256 picks, and the vector is L2-normalised. Texts sharing words land close together
/// under cosine distance, which is all the store's tests need.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Copy)]
pub struct HashEmbedder {
    dim: usize,
}

#[cfg(any(test, feature = "test-support"))]
impl HashEmbedder {
    /// An embedder producing vectors of width `dim` (non-zero).
    #[must_use]
    pub fn new(dim: usize) -> Self {
        assert!(dim > 0, "HashEmbedder needs a non-zero width");
        Self { dim }
    }

    fn vector(&self, text: &str) -> Vec<f32> {
        use sha2::{Digest, Sha256};

        let mut v = vec![0.0_f32; self.dim];
        for word in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            let digest = Sha256::digest(word.to_lowercase().as_bytes());
            let slot = u64::from_le_bytes(digest[..8].try_into().expect("8 bytes")) as usize;
            v[slot % self.dim] += 1.0;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        v
    }
}

#[cfg(any(test, feature = "test-support"))]
impl DenseEmbedder for HashEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, StoreError> {
        Ok(texts.iter().map(|t| self.vector(t)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    /// fastembed's vectors for six texts, recorded once (MOD-68 D1) and read back by every
    /// golden check.
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Goldens {
        provenance: Provenance,
        texts: Vec<String>,
        vectors: Vec<Vec<f32>>,
    }

    /// Where the goldens came from.
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Provenance {
        recorder: String,
        model: String,
        revision: String,
        onnx_sha256: String,
        tokenizer_sha256: String,
        recorded: String,
        tolerance: String,
    }

    /// Read at run time, not `include_str!`, so this module compiles before the recorder has run.
    fn goldens_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bge_small_goldens.json")
    }

    fn read_goldens() -> Goldens {
        let path = goldens_path();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()))
    }

    #[test]
    fn goldens_fixture_is_six_unit_vectors_of_384() {
        let g = read_goldens();
        assert_eq!(g.texts.len(), 6);
        assert_eq!(g.vectors.len(), 6);
        for (i, v) in g.vectors.iter().enumerate() {
            assert_eq!(v.len(), DENSE_DIM, "vector {i}");
            let norm = v
                .iter()
                .map(|&x| f64::from(x) * f64::from(x))
                .sum::<f64>()
                .sqrt();
            assert!((norm - 1.0).abs() <= 1e-6, "vector {i} has norm {norm}");
        }
        assert_eq!(g.texts[4], "");
        assert_eq!(g.texts[5], "MOD-34");
        assert_eq!(
            g.provenance.revision,
            "ea104dacec62c0de699686887e3f920caeb4f3e3"
        );
        assert_eq!(
            g.provenance.onnx_sha256,
            "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35"
        );
    }

    #[tokio::test]
    async fn hash_embedder_is_deterministic_and_sized() {
        let e = HashEmbedder::new(DENSE_DIM);
        let a = e.embed(vec!["Qdrant search".into()]).await.unwrap();
        let b = e.embed(vec!["Qdrant search".into()]).await.unwrap();
        assert_eq!(a, b);
        assert_eq!(a[0].len(), DENSE_DIM);
        assert!((cosine(&a[0], &a[0]) - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    async fn hash_embedder_puts_shared_words_closer() {
        let e = HashEmbedder::new(DENSE_DIM);
        let v = e
            .embed(vec![
                "postgres store migration".into(),
                "postgres store cache".into(),
                "terminal colour theme".into(),
            ])
            .await
            .unwrap();
        assert!(cosine(&v[0], &v[1]) > cosine(&v[0], &v[2]));
    }

    #[tokio::test]
    async fn hash_embedder_handles_empty_text() {
        let v = HashEmbedder::new(8)
            .embed(vec![String::new()])
            .await
            .unwrap();
        assert_eq!(v[0], vec![0.0; 8]);
    }

    #[cfg(feature = "local-embed")]
    #[tokio::test]
    #[ignore = "downloads the BGE-small model"]
    async fn fast_embedder_returns_384_dims() {
        let e = FastEmbedder::new().expect("model loads");
        let v = e.embed(vec!["hello world".into()]).await.expect("embeds");
        assert_eq!(v[0].len(), DENSE_DIM);
    }

    /// The six golden texts: short, a typical item point, one past 512 tokens, Unicode, empty and
    /// the exact key. Only the recorder builds them; every check reads them from the fixture.
    #[cfg(feature = "local-embed")]
    fn golden_texts() -> Vec<String> {
        vec![
            "hello world".to_owned(),
            "MOD-68 Replace fastembed/ort with an rten embedder and a pinned weight fetch. The \
             dense embedder moves to rten plus tokenizers, so a fresh install can load the model \
             and the stored Qdrant vectors stay valid."
                .to_owned(),
            "The store worker serialises every request and answers on its own task. ".repeat(60),
            "検索とベクトル: Qdrant 🦀 naïve café — 数据库 ✅".to_owned(),
            String::new(),
            "MOD-34".to_owned(),
        ]
    }

    #[cfg(feature = "local-embed")]
    #[tokio::test]
    #[ignore = "records the goldens from fastembed; needs the BGE model and HTUI_RECORD_GOLDENS=1"]
    async fn record_fastembed_goldens() {
        if std::env::var("HTUI_RECORD_GOLDENS").as_deref() != Ok("1") {
            eprintln!("HTUI_RECORD_GOLDENS is not 1: the goldens fixture is left as it is");
            return;
        }
        let texts = golden_texts();
        let vectors = FastEmbedder::new()
            .expect("model loads")
            .embed(texts.clone())
            .await
            .expect("embeds");
        assert_eq!(vectors.len(), 6);
        assert!(vectors.iter().all(|v| v.len() == DENSE_DIM));
        let goldens = Goldens {
            provenance: Provenance {
                recorder: "fastembed 3.14.1 (ONNX Runtime via ort), FastEmbedder::embed, one call"
                    .to_owned(),
                model: "Xenova/bge-small-en-v1.5".to_owned(),
                revision: "ea104dacec62c0de699686887e3f920caeb4f3e3".to_owned(),
                onnx_sha256: "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35"
                    .to_owned(),
                tokenizer_sha256:
                    "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66".to_owned(),
                recorded: chrono::Utc::now().date_naive().to_string(),
                tolerance: "1e-5 per element, cosine >= 1 - 1e-9 (ANA-23 §8.1)".to_owned(),
            },
            texts,
            vectors,
        };
        let path = goldens_path();
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("fixtures dir");
        let mut json = serde_json::to_string_pretty(&goldens).expect("serialises");
        json.push('\n');
        std::fs::write(&path, json).expect("writes the fixture");
    }

    /// Every range's padded size, or a lone oversize text.
    fn assert_within_cap(lens: &[usize], cap: usize) {
        for r in sub_batches(lens, cap) {
            let longest = lens[r.clone()].iter().copied().max().unwrap_or(0);
            assert!(
                r.len() * longest <= cap || r.len() == 1,
                "{r:?} pads {} tokens past the cap {cap}",
                r.len() * longest
            );
        }
    }

    /// The ANA-23 §9 shape: a full requirement chunk of short texts and one long one.
    fn chunk_with_one_long_text() -> Vec<usize> {
        let mut lens = vec![20; 255];
        lens.push(512);
        lens
    }

    #[test]
    fn sub_batches_of_nothing_is_nothing() {
        assert!(sub_batches(&[], MAX_BATCH_TOKENS).is_empty());
    }

    #[test]
    fn sub_batches_that_fit_are_one_range() {
        assert_eq!(sub_batches(&[3, 9, 4], 100), vec![0..3]);
    }

    #[test]
    fn sub_batches_honour_the_cap() {
        assert_within_cap(&chunk_with_one_long_text(), MAX_BATCH_TOKENS);
        assert_within_cap(&[512; 40], MAX_BATCH_TOKENS);
        assert_within_cap(&[1, 512, 1, 512, 3, 7, 200], 1024);
        assert_within_cap(&[5, 5, 5, 5, 5], 10);
        assert_within_cap(&[5, 5, 5], 0);
    }

    #[test]
    fn sub_batches_keep_input_order() {
        for (lens, cap) in [
            (chunk_with_one_long_text(), MAX_BATCH_TOKENS),
            (vec![512; 40], MAX_BATCH_TOKENS),
            (vec![1, 512, 1, 512, 3, 7, 200], 1024),
            (vec![7; 3], 0),
        ] {
            let ranges = sub_batches(&lens, cap);
            let mut next = 0;
            for r in &ranges {
                assert_eq!(r.start, next, "{ranges:?}");
                assert!(!r.is_empty(), "{ranges:?}");
                next = r.end;
            }
            assert_eq!(next, lens.len(), "{ranges:?}");
        }
    }

    #[test]
    fn an_oversize_text_is_alone() {
        assert_eq!(sub_batches(&[10, 600, 10], 512), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn a_long_text_does_not_pad_the_whole_chunk() {
        let ranges = sub_batches(&chunk_with_one_long_text(), MAX_BATCH_TOKENS);
        let holding = ranges
            .iter()
            .find(|r| r.contains(&255))
            .expect("a range holds the long text");
        assert!(holding.len() <= 16, "{holding:?} of {ranges:?}");
    }

    /// The pinned files through the production path (B3): on a box with fastembed's old cache
    /// this proves adoption end to end, with no download.
    #[cfg(feature = "local-embed")]
    async fn golden_model() -> ModelFiles {
        model::ensure_model()
            .await
            .expect("the model is fetched or adopted")
    }

    /// Each vector within 1e-5 per element and cosine ≥ 1 - 1e-9 of its golden (D2).
    #[cfg(feature = "local-embed")]
    fn assert_matches_goldens(got: &[Vec<f32>], goldens: &Goldens) {
        assert_eq!(got.len(), goldens.vectors.len());
        let (mut worst_diff, mut worst_cos) = (0.0_f32, 1.0_f64);
        for (i, (a, b)) in got.iter().zip(&goldens.vectors).enumerate() {
            assert_eq!(a.len(), DENSE_DIM, "text {i}");
            let (at, diff) = a
                .iter()
                .zip(b)
                .map(|(x, y)| (x - y).abs())
                .enumerate()
                .fold((0, 0.0_f32), |w, (j, d)| if d > w.1 { (j, d) } else { w });
            assert!(
                diff <= 1e-5,
                "text {i}: element {at} is {} against the golden {} (|diff| {diff})",
                a[at],
                b[at]
            );
            let dot: f64 = a
                .iter()
                .zip(b)
                .map(|(x, y)| f64::from(*x) * f64::from(*y))
                .sum();
            let na = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
            let nb = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
            let cos = dot / (na * nb);
            assert!(cos >= 1.0 - 1e-9, "text {i}: cosine {cos}");
            worst_diff = worst_diff.max(diff);
            worst_cos = worst_cos.min(cos);
        }
        eprintln!("goldens: max element diff {worst_diff:e}, min cosine {worst_cos}");
    }

    #[cfg(feature = "local-embed")]
    #[tokio::test]
    #[ignore = "needs the BGE model (133 MB)"]
    async fn rten_matches_fastembed_goldens() {
        let goldens = read_goldens();
        let e = RtenEmbedder::load(&golden_model().await).expect("model loads");
        let got = e.embed(goldens.texts.clone()).await.expect("embeds");
        assert_matches_goldens(&got, &goldens);
    }

    #[cfg(feature = "local-embed")]
    #[tokio::test]
    #[ignore = "needs the BGE model (133 MB)"]
    async fn rten_matches_goldens_one_text_per_pass() {
        let goldens = read_goldens();
        let e = RtenEmbedder::load(&golden_model().await).expect("model loads");
        let texts = goldens.texts.clone();
        let got = tokio::task::spawn_blocking(move || e.embed_blocking(&texts, model::MAX_TOKENS))
            .await
            .expect("the task ran")
            .expect("embeds");
        assert_matches_goldens(&got, &goldens);
    }

    #[cfg(feature = "local-embed")]
    #[tokio::test]
    #[ignore = "needs the BGE model (133 MB)"]
    async fn rten_embeds_nothing_as_nothing() {
        let e = RtenEmbedder::load(&golden_model().await).expect("model loads");
        assert_eq!(
            e.embed(vec![]).await.expect("embeds"),
            Vec::<Vec<f32>>::new()
        );
    }

    #[cfg(feature = "local-embed")]
    #[test]
    fn rten_load_names_a_missing_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = RtenEmbedder::load(&ModelFiles {
            onnx: tmp.path().join("absent.onnx"),
            tokenizer: tmp.path().join("absent.json"),
        })
        .expect_err("nothing to load");
        assert!(err.to_string().contains("absent.json"), "{err}");
    }
}
