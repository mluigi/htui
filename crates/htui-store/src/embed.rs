//! Dense embeddings for the concepts index (MOD-34, `docs/ANA-20.md` §3.1).
//!
//! [`DenseEmbedder`] is the seam the vector store embeds through. Production uses
//! [`FastEmbedder`] (local ONNX via `fastembed-rs`, behind the `local-embed` feature because
//! `ort` downloads onnxruntime at build time); tests use [`HashEmbedder`], which needs neither a
//! model nor a network. The sparse half of hybrid search is not a model at all: see
//! [`crate::bm25`].
use htui_core::store::StoreError;

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

/// BGE-small-en-v1.5 through `fastembed-rs`, run on the blocking pool. Cheap to clone: the
/// model is shared (MOD-64 D238).
#[cfg(feature = "local-embed")]
#[derive(Clone)]
pub struct FastEmbedder {
    model: std::sync::Arc<fastembed::TextEmbedding>,
}

#[cfg(feature = "local-embed")]
impl std::fmt::Debug for FastEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            model: std::sync::Arc::new(model),
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
}
