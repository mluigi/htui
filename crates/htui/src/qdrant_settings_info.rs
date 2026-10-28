use chrono::{DateTime, Utc};
use htui_core::qdrant::{QdrantUrl, QdrantUrlError};
use htui_core::store::{CasOutcome, Result, StoreError};
use htui_store::Backend;
use htui_store::secret::QdrantEnv;

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

/// Represents the presence or readability of this box's Qdrant API key in its keyring.
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

/// MOD-158 D9: the database-wide `qdrant_url` row as the Settings section shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlRowState {
    /// A row that parses, with its compare-and-set token.
    Set {
        /// The URL every box uses.
        url: QdrantUrl,
        /// `app_setting.updated_at`, presented by a write.
        token: DateTime<Utc>,
    },
    /// A row that does not parse (blueprint P-3): it can still be overwritten or cleared.
    Unusable {
        /// Why the stored value is not a Qdrant URL.
        why: QdrantUrlError,
        /// `app_setting.updated_at`, presented by a write.
        token: DateTime<Utc>,
    },
    /// No row: no box has a Qdrant URL.
    NotSet,
    /// `Backend::Offline`: the row lives in Postgres, which is not connected.
    Offline,
    /// The read failed with something other than a lost server.
    Unreadable(String),
    /// `Backend::Memory` (`--demo`): no database to read.
    NotApplicable,
}

impl UrlRowState {
    /// The row's compare-and-set token, when there is a row.
    #[must_use]
    pub const fn token(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Set { token, .. } | Self::Unusable { token, .. } => Some(*token),
            Self::NotSet | Self::Offline | Self::Unreadable(_) | Self::NotApplicable => None,
        }
    }
}

/// MOD-158 D10: this box's retired keyring `qdrant-url` entry, as D2 reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OldUrl {
    /// A URL that passes D2: it can be used for every box, or discarded.
    Usable(QdrantUrl),
    /// A value that does not: it can only be discarded.
    Unusable(QdrantUrlError),
}

/// A snapshot of the Qdrant settings this process sees (MOD-158 D9, D10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QdrantSnapshot {
    /// The database-wide `qdrant_url` row (D9).
    pub url: UrlRowState,
    /// `HTUI_QDRANT_URL` in force in this process (D3), parsed. `None` under `--demo`.
    pub url_override: Option<core::result::Result<QdrantUrl, QdrantUrlError>>,
    /// This box's keyring `qdrant-key`.
    pub key_state: QdrantState,
    /// `HTUI_QDRANT_API_KEY` is set in this process. `false` under `--demo`.
    pub key_override: bool,
    /// D10: this box's keyring `qdrant-url`, retired. `None` when absent, unreadable, or under
    /// `--demo`.
    pub old_url: Option<OldUrl>,
}

impl QdrantSnapshot {
    /// A demo session's: nothing read, every row not applicable (CLEAN-8 #9).
    #[must_use]
    pub const fn not_applicable() -> Self {
        Self {
            url: UrlRowState::NotApplicable,
            url_override: None,
            key_state: QdrantState::NotApplicable,
            key_override: false,
            old_url: None,
        }
    }

    /// Reads the `qdrant_url` row through `backend`, `env`'s overrides, and this box's keyring
    /// entries (each under `spawn_blocking`).
    ///
    /// # Errors
    ///
    /// [`StoreError::Unreachable`] when the server is lost during the row's read, so the loop drops
    /// offline; every other read failure is a row or entry state, not an error.
    pub async fn fetch(backend: &Backend, env: &QdrantEnv) -> Result<Self> {
        if matches!(backend, Backend::Memory(_)) {
            return Ok(Self::not_applicable());
        }
        let url = url_row(backend).await?;
        let key_state = match blocking_keyring(htui_store::secret::get_qdrant_api_key).await {
            Ok(Some(_)) => QdrantState::Stored,
            Ok(None) => QdrantState::NotStored,
            Err(e) => QdrantState::Unreadable(e.to_string()),
        };
        let old_url = match blocking_keyring(htui_store::secret::get_qdrant_url).await {
            Ok(Some(text)) => Some(match QdrantUrl::parse(&text) {
                Ok(url) => OldUrl::Usable(url),
                Err(why) => OldUrl::Unusable(why),
            }),
            Ok(None) | Err(_) => None,
        };
        Ok(Self {
            url,
            url_override: QdrantUrl::from_env(env.url.as_deref()).transpose(),
            key_state,
            key_override: env.api_key.is_some(),
            old_url,
        })
    }
}

/// The `qdrant_url` row as [`UrlRowState`].
async fn url_row(backend: &Backend) -> Result<UrlRowState> {
    let pg = match backend {
        Backend::Memory(_) => return Ok(UrlRowState::NotApplicable),
        Backend::Offline { .. } => return Ok(UrlRowState::Offline),
        Backend::Online { pg, .. } => pg,
    };
    Ok(match pg.qdrant_url().await {
        Ok(Some(row)) => match row.url {
            Ok(url) => UrlRowState::Set {
                url,
                token: row.token,
            },
            Err(why) => UrlRowState::Unusable {
                why,
                token: row.token,
            },
        },
        Ok(None) => UrlRowState::NotSet,
        Err(err @ StoreError::Unreachable(_)) => return Err(err),
        Err(err) => UrlRowState::Unreadable(err.to_string()),
    })
}

/// The seven Settings > Qdrant request names, in [`StoreRequest`] order (MOD-158 D9, D10).
pub const REQUEST_NAMES: [&str; 7] = [
    "qdrant_info",
    "set_qdrant_url",
    "clear_qdrant_url",
    "set_qdrant_api_key",
    "clear_qdrant_api_key",
    "adopt_old_qdrant_url",
    "discard_old_qdrant_url",
];

/// What a URL write (and the adopt, which is one) answers on `Backend::Offline`.
pub const URL_WRITE_OFFLINE: &str = "the Qdrant URL cannot be changed while Postgres is offline";

/// What the adopt answers when this box's keyring holds no old URL.
pub const NO_OLD_URL: &str = "this box's keyring holds no old Qdrant URL";

/// What the adopt answers when the old URL fails D2: it can only be discarded.
#[must_use]
pub fn old_url_unusable(why: QdrantUrlError) -> String {
    format!("the old Qdrant URL is not usable: {why}; discard it")
}

/// What the adopt answers when the row landed and the keyring entry stayed.
#[must_use]
pub fn old_entry_kept(err: &StoreError) -> String {
    format!("the URL is set for every box, but the old keyring entry could not be removed: {err}")
}

/// A `Failed` reply to `request`.
fn failed(request: &StoreRequest, message: impl Into<String>) -> StoreReply {
    StoreReply::Failed {
        request: request.name(),
        message: message.into(),
    }
}

/// A compare-and-set outcome as a reply: the fresh snapshot, under `QdrantStale` on a miss.
async fn cas<T>(backend: &Backend, env: &QdrantEnv, outcome: &CasOutcome<T>) -> Result<StoreReply> {
    let fresh = QdrantSnapshot::fetch(backend, env).await?;
    Ok(match outcome {
        CasOutcome::Applied(_) => StoreReply::Qdrant(fresh),
        CasOutcome::Stale(_) => StoreReply::QdrantStale(Box::new(fresh)),
    })
}

/// CLEAN-8 #9, MOD-158 D9: the seven Settings > Qdrant requests, for `try_serve` (the loop's
/// `other` arm, the harness and `--demo` alike), with this process's overrides. On
/// [`Backend::Memory`] no environment is read (see [`serve_with`]).
///
/// # Errors
///
/// As [`serve_with`].
pub(crate) async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let env = if matches!(backend, Backend::Memory(_)) {
        QdrantEnv::default()
    } else {
        // Blueprint P-13: the production entry point is the only reader of the process's.
        QdrantEnv::from_process()
    };
    serve_with(backend, request, &env).await
}

/// The seven requests over `backend` with the overrides `env` (a test's literal, blueprint P-13).
///
/// On [`Backend::Memory`] the read is [`QdrantSnapshot::not_applicable`] and every write is
/// refused with [`DEMO_SESSION`] before a keyring or a store is reached. On [`Backend::Offline`]
/// the URL writes and the adopt are refused with [`URL_WRITE_OFFLINE`]; the key writes and the
/// discard, which touch this box's keyring only, work.
///
/// # Errors
///
/// A keyring call's or the row's [`StoreError`], which the caller renders under the request's
/// name ([`StoreError::Unreachable`] drops the loop offline); a failed join; and
/// [`StoreError::Backend`] for a request that is not one of the seven.
pub async fn serve_with(
    backend: &Backend,
    request: &StoreRequest,
    env: &QdrantEnv,
) -> Result<StoreReply> {
    if matches!(backend, Backend::Memory(_)) {
        return Ok(match request {
            StoreRequest::QdrantInfo => StoreReply::Qdrant(QdrantSnapshot::not_applicable()),
            StoreRequest::SetQdrantUrl { .. }
            | StoreRequest::ClearQdrantUrl { .. }
            | StoreRequest::SetQdrantApiKey(_)
            | StoreRequest::ClearQdrantApiKey
            | StoreRequest::AdoptOldQdrantUrl
            | StoreRequest::DiscardOldQdrantUrl => failed(request, DEMO_SESSION),
            other => return Err(not_ours(other)),
        });
    }
    let pg = backend.writable();
    Ok(match request {
        StoreRequest::QdrantInfo => StoreReply::Qdrant(QdrantSnapshot::fetch(backend, env).await?),
        StoreRequest::SetQdrantUrl { url, expected } => {
            let Some(pg) = pg else {
                return Ok(failed(request, URL_WRITE_OFFLINE));
            };
            let outcome = pg.set_qdrant_url(url, *expected).await?;
            cas(backend, env, &outcome).await?
        }
        StoreRequest::ClearQdrantUrl { expected } => {
            let Some(pg) = pg else {
                return Ok(failed(request, URL_WRITE_OFFLINE));
            };
            let outcome = pg.clear_qdrant_url(*expected).await?;
            cas(backend, env, &outcome).await?
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
            StoreReply::Qdrant(QdrantSnapshot::fetch(backend, env).await?)
        }
        StoreRequest::ClearQdrantApiKey => {
            blocking_keyring(htui_store::secret::clear_qdrant_api_key).await?;
            StoreReply::Qdrant(QdrantSnapshot::fetch(backend, env).await?)
        }
        StoreRequest::AdoptOldQdrantUrl => {
            let Some(pg) = pg else {
                return Ok(failed(request, URL_WRITE_OFFLINE));
            };
            // The worker re-reads the entry rather than trusting a snapshot's copy of it.
            let Some(text) = blocking_keyring(htui_store::secret::get_qdrant_url).await? else {
                return Ok(failed(request, NO_OLD_URL));
            };
            let url = match QdrantUrl::parse(&text) {
                Ok(url) => url,
                Err(why) => return Ok(failed(request, old_url_unusable(why))),
            };
            // D10: the insert expecting no row; a row already there is `Stale`, and the entry is
            // kept, so the offer reads discard-only (blueprint R-4).
            match pg.set_qdrant_url(&url, None).await? {
                CasOutcome::Stale(_) => {
                    StoreReply::QdrantStale(Box::new(QdrantSnapshot::fetch(backend, env).await?))
                }
                CasOutcome::Applied(_) => {
                    if let Err(err) = blocking_keyring(htui_store::secret::clear_qdrant_url).await {
                        return Ok(failed(request, old_entry_kept(&err)));
                    }
                    StoreReply::Qdrant(QdrantSnapshot::fetch(backend, env).await?)
                }
            }
        }
        StoreRequest::DiscardOldQdrantUrl => {
            blocking_keyring(htui_store::secret::clear_qdrant_url).await?;
            StoreReply::Qdrant(QdrantSnapshot::fetch(backend, env).await?)
        }
        other => return Err(not_ours(other)),
    })
}

/// A request `try_serve` never routes here, named rather than panicked on.
fn not_ours(request: &StoreRequest) -> StoreError {
    StoreError::Backend(format!("not a qdrant request: {}", request.name()))
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

    /// The seven names are the `name()` arms, in [`StoreRequest`] order (`connection.rs`'s rule).
    #[test]
    fn request_names_match_the_name_arms() {
        let url = QdrantUrl::parse("http://q.example:6334").expect("a valid URL");
        let token = DateTime::<Utc>::UNIX_EPOCH;
        let requests = [
            StoreRequest::QdrantInfo,
            StoreRequest::SetQdrantUrl {
                url,
                expected: None,
            },
            StoreRequest::ClearQdrantUrl { expected: token },
            StoreRequest::SetQdrantApiKey(crate::secrets_settings::Redacted::new(String::new())),
            StoreRequest::ClearQdrantApiKey,
            StoreRequest::AdoptOldQdrantUrl,
            StoreRequest::DiscardOldQdrantUrl,
        ];
        let names: Vec<&str> = requests.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
    }

    /// A demo session reads nothing and refuses every write before a keyring is reached.
    #[tokio::test]
    async fn a_demo_backend_answers_not_applicable_and_refuses_writes() {
        let demo = Backend::memory(htui_core::store::MemStore::demo());
        let env = QdrantEnv::new(Some("http://env.example:6334".to_owned()), None);
        match serve_with(&demo, &StoreRequest::QdrantInfo, &env).await {
            Ok(StoreReply::Qdrant(snapshot)) => {
                assert_eq!(snapshot, QdrantSnapshot::not_applicable());
            }
            other => panic!("a not-applicable snapshot, got {other:?}"),
        }
        match serve_with(&demo, &StoreRequest::DiscardOldQdrantUrl, &env).await {
            Ok(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "discard_old_qdrant_url");
                assert_eq!(message, DEMO_SESSION);
            }
            other => panic!("a demo refusal, got {other:?}"),
        }
    }
}
