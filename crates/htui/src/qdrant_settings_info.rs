use htui_core::store::{Result, StoreError};
use htui_store::Backend;

use crate::secrets_settings::DEMO_SESSION;
use crate::store_worker::{StoreReply, StoreRequest};

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
    /// `Backend::Memory`: no keyring consulted (CLEAN-8 #9, D6).
    NotApplicable,
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

/// CLEAN-8 #9: the four Settings > Qdrant requests, for `try_serve` (the loop's `other` arm, the
/// harness and `--demo` alike). On [`Backend::Memory`] the read is [`QdrantState::NotApplicable`]
/// and every write is refused with [`DEMO_SESSION`] before the keyring is reached, as the Secrets
/// section's are.
///
/// # Errors
///
/// A keyring call's [`StoreError::Backend`], which the caller renders under the request's name;
/// a failed join; and [`StoreError::Backend`] for a request that is not one of the four, which
/// `try_serve` never sends here.
pub(crate) async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let demo = matches!(backend, Backend::Memory(_));
    Ok(match request {
        StoreRequest::QdrantInfo if demo => StoreReply::Qdrant(QdrantSnapshot {
            url_state: QdrantState::NotApplicable,
            key_state: QdrantState::NotApplicable,
            url_summary: None,
        }),
        StoreRequest::SetQdrantUrl(_)
        | StoreRequest::SetQdrantApiKey(_)
        | StoreRequest::ClearQdrantSettings
            if demo =>
        {
            StoreReply::Failed {
                request: request.name(),
                message: DEMO_SESSION.to_owned(),
            }
        }
        StoreRequest::QdrantInfo => StoreReply::Qdrant(QdrantSnapshot::fetch().await),
        StoreRequest::SetQdrantUrl(url) => {
            let url = url.clone();
            blocking_keyring(move || htui_store::secret::set_qdrant_url(&url)).await?;
            StoreReply::Qdrant(QdrantSnapshot::fetch().await)
        }
        StoreRequest::SetQdrantApiKey(key) => {
            // A zeroizing clone into the closure; nothing unzeroized (MOD-10 M4 D9).
            let key = key.clone();
            blocking_keyring(move || {
                if key.expose().is_empty() {
                    htui_store::secret::clear_qdrant_api_key()
                } else {
                    htui_store::secret::set_qdrant_api_key(key.expose())
                }
            })
            .await?;
            StoreReply::Qdrant(QdrantSnapshot::fetch().await)
        }
        StoreRequest::ClearQdrantSettings => {
            blocking_keyring(|| {
                htui_store::secret::clear_qdrant_url()?;
                htui_store::secret::clear_qdrant_api_key()
            })
            .await?;
            StoreReply::Qdrant(QdrantSnapshot::fetch().await)
        }
        other => {
            return Err(StoreError::Backend(format!(
                "not a qdrant request: {}",
                other.name()
            )));
        }
    })
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
