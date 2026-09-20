/// Represents the presence or readability of a Qdrant setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QdrantState {
    /// The setting is stored and successfully read.
    Stored,
    /// The setting is not present in the keyring.
    NotStored,
    /// The setting could not be read from the keyring.
    Unreadable(String),
}

/// A snapshot of the current Qdrant settings in the OS keyring.
#[derive(Debug, Clone)]
pub struct QdrantSnapshot {
    /// The state of the Qdrant URL.
    pub url_state: QdrantState,
    /// The state of the Qdrant API key.
    pub key_state: QdrantState,
    /// A masked summary of the URL.
    pub url_summary: Option<String>,
}

impl QdrantSnapshot {
    /// Fetches the current Qdrant settings from the keyring.
    pub async fn fetch() -> Self {
        let url_res = tokio::task::spawn_blocking(htui_store::secret::get_qdrant_url)
            .await
            .unwrap();
        let key_res = tokio::task::spawn_blocking(htui_store::secret::get_qdrant_api_key)
            .await
            .unwrap();

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
