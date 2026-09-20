use crate::vector::VectorStore;
use htui_core::store::StoreError;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use sha2::{Sha256, Digest};
use tokio::fs;

/// A background worker that synchronizes `HANDOFF.md` and `docs/` markdown files to a `VectorStore`.
#[derive(Debug)]
pub struct VectorSync {
    repo_root: PathBuf,
}

impl VectorSync {
    /// Create a new VectorSync
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        Self {
            repo_root: repo_root.as_ref().to_path_buf(),
        }
    }

    /// Iterates `HANDOFF.md` and `docs/` and pushes their contents to the vector store.
    pub async fn run_once(&self, store: &impl VectorStore) -> Result<(), StoreError> {
        self.sync_docs(store).await?;
        self.sync_handoff(store).await?;
        Ok(())
    }

    async fn sync_docs(&self, store: &impl VectorStore) -> Result<(), StoreError> {
        let docs_dir = self.repo_root.join("docs");
        if !docs_dir.exists() {
            return Ok(());
        }

        let docs_dir_clone = docs_dir.clone();
        let entries = tokio::task::spawn_blocking(move || {
            WalkDir::new(docs_dir_clone)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
                .map(|e| e.path().to_path_buf())
                .collect::<Vec<_>>()
        }).await.map_err(|e| StoreError::Backend(e.to_string()))?;

        for path in entries {
            let content = fs::read_to_string(&path)
                .await
                .map_err(|e| StoreError::Backend(format!("Failed to read {:?}: {}", path, e)))?;
            
            // Generate deterministic ID from relative path
            let id = path.strip_prefix(&self.repo_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            let mut meta = serde_json::Map::new();
            meta.insert(
                "path".to_string(),
                serde_json::Value::String(id.clone()),
            );
            
            // Upsert documents in 512-token safe chunks (naive character chunking for now)
            let chunks = self.chunk_text(&content, 2000);
            for (i, chunk) in chunks.iter().enumerate() {
                let chunk_id = format!("{}-{}", id, i);
                store.upsert_document(&chunk_id, chunk, serde_json::Value::Object(meta.clone())).await?;
            }
        }
        Ok(())
    }

    async fn sync_handoff(&self, store: &impl VectorStore) -> Result<(), StoreError> {
        let handoff_path = self.repo_root.join("HANDOFF.md");
        if !handoff_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&handoff_path)
            .await
            .map_err(|e| StoreError::Backend(format!("Failed to read HANDOFF.md: {}", e)))?;
        
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        let mut meta = serde_json::Map::new();
        meta.insert(
            "hash".to_string(),
            serde_json::Value::String(hash.clone()),
        );

        // Splitting HANDOFF into chunks
        let chunks = self.chunk_text(&content, 2000);
        for (i, chunk) in chunks.iter().enumerate() {
            let chunk_id = format!("HANDOFF-{}", i);
            store.upsert_item(&chunk_id, chunk, serde_json::Value::Object(meta.clone())).await?;
        }
        Ok(())
    }

    fn chunk_text(&self, text: &str, size: usize) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();

        for line in text.lines() {
            if current_chunk.len() + line.len() > size && !current_chunk.is_empty() {
                chunks.push(current_chunk.clone());
                current_chunk.clear();
            }
            current_chunk.push_str(line);
            current_chunk.push('\n');
        }

        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }
        chunks
    }
}
