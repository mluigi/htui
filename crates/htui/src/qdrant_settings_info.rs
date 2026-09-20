
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QdrantState {
    Stored,
    NotStored,
    Unreadable(String),
}

#[derive(Debug, Clone)]
pub struct QdrantSnapshot {
    pub url_state: QdrantState,
    pub key_state: QdrantState,
    pub url_summary: Option<String>,
}

impl QdrantSnapshot {
    pub async fn fetch() -> Self {
        let url_res = tokio::task::spawn_blocking(htui_store::secret::get_qdrant_url).await.unwrap();
        let key_res = tokio::task::spawn_blocking(htui_store::secret::get_qdrant_api_key).await.unwrap();

        let (url_state, url_summary) = match url_res {
            Ok(Some(u)) => (QdrantState::Stored, Some(u)),
            Ok(None) => (QdrantState::NotStored, None),
            Err(e) => (QdrantState::Unreadable(e.to_string()), None),
        };

        let key_state = match key_res {
            Ok(Some(_)) => QdrantState::Stored,
            Ok(None) => QdrantState::NotStored,
            Err(e) => QdrantState::Unreadable(e.to_string()),
        };

        Self {
            url_state,
            key_state,
            url_summary,
        }
    }
}
