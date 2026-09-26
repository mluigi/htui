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

/// BGE-small-en-v1.5 through `fastembed-rs`, run on the blocking pool.
#[cfg(feature = "local-embed")]
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
}
