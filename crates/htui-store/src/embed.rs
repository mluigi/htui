//! Provides types for embedding documents.
use fastembed::{EmbeddingModel, InitOptions, SparseEmbedding, SparseTextEmbedding, TextEmbedding};
use htui_core::store::StoreError;
use std::sync::Arc;

/// Wrapper for local embedding generation using fastembed-rs.
pub struct Embedder {
    dense_model: Arc<TextEmbedding>,
    sparse_model: Arc<SparseTextEmbedding>,
}

impl std::fmt::Debug for Embedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Embedder").finish_non_exhaustive()
    }
}

impl Embedder {
    /// Initialize the local embedder (downloads models if necessary).
    pub fn new() -> Result<Self, StoreError> {
        let dense_model = TextEmbedding::try_new(InitOptions {
            model_name: EmbeddingModel::BGESmallENV15,
            show_download_progress: false,
            ..Default::default()
        })
        .map_err(|e| StoreError::Backend(format!("Failed to load dense model: {}", e)))?;

        let sparse_model = SparseTextEmbedding::try_new(Default::default())
            .map_err(|e| StoreError::Backend(format!("Failed to load sparse model: {}", e)))?;

        Ok(Self {
            dense_model: Arc::new(dense_model),
            sparse_model: Arc::new(sparse_model),
        })
    }

    /// Generate dense embeddings for the given texts.
    pub async fn embed_dense(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, StoreError> {
        let model = self.dense_model.clone();
        tokio::task::spawn_blocking(move || {
            model
                .embed(texts, None)
                .map_err(|e| StoreError::Backend(e.to_string()))
        })
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?
    }

    /// Generate sparse (BM25) embeddings for the given texts.
    pub async fn embed_sparse(
        &self,
        texts: Vec<String>,
    ) -> Result<Vec<SparseEmbedding>, StoreError> {
        let model = self.sparse_model.clone();
        tokio::task::spawn_blocking(move || {
            model
                .embed(texts, None)
                .map_err(|e| StoreError::Backend(e.to_string()))
        })
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires internet access to download model"]
    fn test_dense_embedder() {
        // Initialize rustls provider for reqwest/ureq inside fastembed/hf-hub
        let _ = rustls::crypto::ring::default_provider().install_default();

        let embedder = Embedder::new().expect("Failed to init embedder");

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let embeddings = embedder
                .embed_dense(vec!["hello world".to_string()])
                .await
                .expect("Failed to embed");
            assert_eq!(embeddings.len(), 1);
            assert_eq!(embeddings[0].len(), 384); // bge-small is 384 dims
        });
    }
}
