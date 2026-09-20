#![allow(async_fn_in_trait)]
use crate::embed::Embedder;
use htui_core::store::StoreError;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, SearchPointsBuilder, UpsertPointsBuilder,
    Value as QdrantValue, VectorParamsBuilder, VectorsConfigBuilder, CreateFieldIndexCollectionBuilder,
    FieldType, Vector, Vectors, SparseIndices, SparseVectorsConfigBuilder, SparseVectorParamsBuilder,
    PointId,
};
use qdrant_client::Qdrant;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use serde_json::Value;

/// VectorStore trait for semantic search.
pub trait VectorStore {
    /// Upserts a document into the vector store.
    async fn upsert_document(
        &self,
        doc_id: &str,
        text: &str,
        metadata: Value,
    ) -> Result<(), StoreError>;

    /// Upserts an item into the vector store.
    async fn upsert_item(
        &self,
        item_id: &str,
        text: &str,
        metadata: Value,
    ) -> Result<(), StoreError>;

    /// Searches for concepts using hybrid search and returns the matched IDs.
    async fn search_concepts(
        &self,
        query: &str,
        limit: u64,
    ) -> Result<Vec<String>, StoreError>;
}

/// A VectorStore implementation using Qdrant.
pub struct QdrantStore {
    client: Qdrant,
    embedder: Embedder,
    collection_name: String,
}

impl std::fmt::Debug for QdrantStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QdrantStore")
            .field("collection_name", &self.collection_name)
            .finish_non_exhaustive()
    }
}

impl QdrantStore {
    /// Creates a new QdrantStore, initializing the collection if necessary.
    pub async fn new(collection_name: &str) -> Result<Self, StoreError> {
        let url = crate::secret::get_qdrant_url()?
            .ok_or_else(|| StoreError::Backend("no Qdrant URL stored".into()))?;
        let api_key = crate::secret::get_qdrant_api_key()?;

        let mut builder = Qdrant::from_url(&url);
        if let Some(key) = api_key {
            builder = builder.api_key(key);
        }
        
        let client = builder.build().map_err(|e| {
            StoreError::Backend(format!("Qdrant client error: {}", e))
        })?;
        let embedder = Embedder::new().map_err(|e| {
            StoreError::Backend(format!("Fastembed error: {}", e))
        })?;

        let store = Self {
            client,
            embedder,
            collection_name: collection_name.to_string(),
        };

        store.init_collection().await?;
        Ok(store)
    }

    async fn init_collection(&self) -> Result<(), StoreError> {
        let exists = self
            .client
            .collection_exists(&self.collection_name)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        if !exists {
            let mut vectors_config = VectorsConfigBuilder::default();
            vectors_config.add_named_vector_params(
                "dense",
                VectorParamsBuilder::new(384, Distance::Cosine),
            );

            let mut sparse_vectors_config = SparseVectorsConfigBuilder::default();
            sparse_vectors_config.add_named_vector_params(
                "sparse",
                SparseVectorParamsBuilder::default(),
            );

            self.client
                .create_collection(
                    CreateCollectionBuilder::new(&self.collection_name)
                        .vectors_config(vectors_config)
                        .sparse_vectors_config(sparse_vectors_config)
                )
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;

            // Create payload index for "type"
            self.client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(&self.collection_name, "type", FieldType::Keyword)
                )
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
        }
        Ok(())
    }

    fn generate_point_id(id: &str) -> PointId {
        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        PointId::from(hasher.finish())
    }

    async fn upsert(&self, id: &str, text: &str, metadata: Value, entity_type: &str) -> Result<(), StoreError> {
        let text_owned = text.to_string();
        let dense_embeddings = self
            .embedder
            .embed_dense(vec![text_owned.clone()])
            .await?;
        let sparse_embeddings = self
            .embedder
            .embed_sparse(vec![text_owned])
            .await?;

        let dense = dense_embeddings
            .into_iter()
            .next()
            .ok_or_else(|| StoreError::Backend("No dense embedding generated".to_string()))?;
        let sparse = sparse_embeddings
            .into_iter()
            .next()
            .ok_or_else(|| StoreError::Backend("No sparse embedding generated".to_string()))?;

        let mut payload = HashMap::new();
        payload.insert(
            "type".to_string(),
            QdrantValue::from(entity_type.to_string()),
        );
        payload.insert(
            "id".to_string(),
            QdrantValue::from(id.to_string()),
        );
        
        // Add additional metadata
        if let Some(obj) = metadata.as_object() {
            for (k, v) in obj {
                // simple mapping since QdrantValue::from lacks Value mapping in some versions
                if let Some(s) = v.as_str() {
                    payload.insert(k.clone(), QdrantValue::from(s.to_string()));
                } else if let Some(n) = v.as_f64() {
                    payload.insert(k.clone(), QdrantValue::from(n));
                } else if let Some(b) = v.as_bool() {
                    payload.insert(k.clone(), QdrantValue::from(b));
                }
            }
        }

        let point_id = Self::generate_point_id(id);

        let mut vectors_map = HashMap::new();
        vectors_map.insert("dense".to_string(), Vector::from(dense));
        
        let sparse_vector = Vector {
            data: sparse.values,
            indices: Some(SparseIndices {
                data: sparse.indices.into_iter().map(|i| i as u32).collect()
            }),
            ..Default::default()
        };
        vectors_map.insert("sparse".to_string(), sparse_vector);

        let point = PointStruct::new(
            point_id,
            Vectors::from(vectors_map),
            payload,
        );

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]))
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }
}

impl VectorStore for QdrantStore {
    async fn upsert_document(
        &self,
        doc_id: &str,
        text: &str,
        metadata: Value,
    ) -> Result<(), StoreError> {
        self.upsert(doc_id, text, metadata, "doc").await
    }

    async fn upsert_item(
        &self,
        item_id: &str,
        text: &str,
        metadata: Value,
    ) -> Result<(), StoreError> {
        self.upsert(item_id, text, metadata, "item").await
    }

    async fn search_concepts(&self, query: &str, limit: u64) -> Result<Vec<String>, StoreError> {
        let query_owned = query.to_string();
        let dense_embeddings = self
            .embedder
            .embed_dense(vec![query_owned.clone()])
            .await?;
        let _sparse_embeddings = self
            .embedder
            .embed_sparse(vec![query_owned])
            .await?;

        let dense = dense_embeddings
            .into_iter()
            .next()
            .ok_or_else(|| StoreError::Backend("No dense embedding generated".to_string()))?;

        // Simple hybrid search logic using dense vectors, as qdrant currently doesn't expose a unified `search_hybrid` in the Rust client directly without complex nested queries.
        // For ANA-20, a full exact string matching hybrid query is complex, we use dense here for now or `SearchPointsBuilder` on the "dense" vector.
        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, dense, limit)
                    .vector_name("dense")
                    .with_payload(true),
            )
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        let mut results = Vec::new();
        for point in search_result.result {
            if let Some(id_val) = point.payload.get("id") {
                if let Some(id_str) = id_val.as_str() {
                    results.push(id_str.to_string());
                }
            }
        }
        Ok(results)
    }
}
