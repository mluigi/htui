use htui_core::store::{Result, StoreError};

/// Runs one Qdrant keyring call on a blocking thread. A task that fails to join (it panicked) is a
/// [`StoreError::Backend`], as in `secrets_settings`, so it never panics the store loop that
/// awaits it (MOD-10 M4 R1 L-8).
///
/// # Errors
///
/// What `call` returns, or [`StoreError::Backend`] when the task fails to join.
pub(crate) async fn blocking_keyring<T, F>(call: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))?
}

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
        let url_res = blocking_keyring(htui_store::secret::get_qdrant_url).await;
        let key_res = blocking_keyring(htui_store::secret::get_qdrant_api_key).await;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-10 M4 R1 L-8: a keyring task that panics is a `Backend` error, not a panic on the
    /// store loop that awaits it.
    #[tokio::test]
    async fn a_panicking_keyring_task_is_a_backend_error() {
        let res = blocking_keyring(|| -> Result<()> { panic!("boom") }).await;
        match res {
            Err(StoreError::Backend(message)) => {
                assert!(message.starts_with("keyring task failed"), "{message}");
            }
            other => panic!("a join failure is a Backend error, got {other:?}"),
        }
        assert_eq!(blocking_keyring(|| Ok(7)).await.expect("joins"), 7);
    }
}
