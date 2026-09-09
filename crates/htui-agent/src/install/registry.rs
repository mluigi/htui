//! The ACP registry document, its cache, and the one read the pre-flight makes (plan MOD-20 D12).
//!
//! The document is public, mutable and unversioned beyond a `version` string: it is fetched from
//! a CDN with `max-age=300` and no pinned snapshot URL, and it grew fields between the day this
//! item was written and the day it ships. So every type here is **defensive** — each optional
//! field carries `#[serde(default)]`, unknown keys are ignored (serde's default), and a platform
//! that is simply absent is a first-class answer rather than a parse failure. A registry that
//! sprouts a key must never be the reason a box cannot install.
//!
//! Nothing here knows an agent's name (`R-AGT-5`): a row hands over an `id`, this module looks it
//! up, and the document supplies every other coordinate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::http::{HttpClient, RegistryFetch};
use super::{InstallConfig, REGISTRY_DEFAULT_MAX_AGE};

/// The file the cached document is written to, under [`RegistryCache`]'s directory.
const CACHE_BODY: &str = "latest.json";
/// The file the cached document's headers are written to.
const CACHE_META: &str = "latest.meta.json";

/// `registry.json`, as much of it as this item reads.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistryDocument {
    /// The document's own schema version, `1.0.0` today. Recorded, not enforced: a bump this
    /// code has not seen is not a reason to refuse a row whose entry still parses.
    pub version: String,
    /// Every agent the registry lists.
    pub agents: Vec<RegistryAgent>,
}

impl RegistryDocument {
    /// The entry a row's `install.id` names, if the document lists it.
    #[must_use]
    pub fn agent(&self, id: &str) -> Option<&RegistryAgent> {
        self.agents.iter().find(|agent| agent.id == id)
    }
}

/// One `agents[]` entry: what to fetch for each platform, and on what terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryAgent {
    /// The entry id — the only field a seed row spells, and the only one that is required.
    pub id: String,
    /// The display name the consent pane shows.
    #[serde(default)]
    pub name: String,
    /// The version the registry serves. It publishes `latest` only, which is why a *lower*
    /// version arriving here is the downgrade hazard H-15 refuses rather than an ordinary case.
    #[serde(default)]
    pub version: String,
    /// The licence identifier, `proprietary` for entries that have no SPDX id.
    #[serde(default)]
    pub license: Option<String>,
    /// Where the terms are, so consent is given against something the user can read.
    #[serde(default)]
    pub license_url: Option<String>,
    /// The per-platform archives.
    #[serde(default)]
    pub distribution: Distribution,
}

/// `distribution` — one map today, keyed by `<os>-<arch>`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Distribution {
    /// `binary`, keyed by [`crate::probe::platform_key`]'s vocabulary.
    pub binary: BTreeMap<String, BinaryEntry>,
}

/// One platform's archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryEntry {
    /// The archive URL. Required: an entry without one describes nothing.
    pub archive: String,
    /// The command inside the unpacked archive, as the registry spells it — `./name` on the unix
    /// platforms, a bare `name.exe` on some Windows ones. Both spellings are relative, which is
    /// all `install::plan::cmd_relative` insists on.
    pub cmd: String,
    /// Arguments the command needs on this platform, appended by the row's `PlatformGlob`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment the command needs on this platform.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Lowercase hex sha256 of the archive, when the entry publishes one. Eight of the registry's
    /// forty entries do not, which is why this is an `Option` and not a requirement.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Where the document the pre-flight used came from, for the consent text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrySource {
    /// Fetched now, or served from a cache the CDN's own `max-age` still calls current. Either
    /// way the consent pane says nothing: the user is looking at what the registry says today.
    Fresh,
    /// The network failed and this is what the box had. The age goes into the consent text, so a
    /// plan built from a week-old document says so before the user presses `y`.
    Cached {
        /// How long ago the cached copy was fetched.
        age: Duration,
    },
}

/// The headers worth keeping beside a cached document (plan D12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CacheMeta {
    /// The `ETag` the next conditional `GET` sends.
    pub(crate) etag: Option<String>,
    /// When the body was last known current — moved forward by a `304` as well as by a body.
    pub(crate) fetched_at: DateTime<Utc>,
    /// The CDN's `max-age`, or [`REGISTRY_DEFAULT_MAX_AGE`] when it sent none.
    pub(crate) max_age_secs: u64,
}

/// `<install_root>/.registry/` — a sibling of the `<id>/` directories, never inside one.
///
/// The placement is load-bearing (hazard H-3): a seed's glob has its literal root at
/// `<root>/<id>`, so nothing under a dot-directory beside it can ever be resolved as an adapter.
#[derive(Debug)]
pub(crate) struct RegistryCache {
    dir: PathBuf,
}

impl RegistryCache {
    /// The cache under one install root.
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            dir: root.join(".registry"),
        }
    }

    /// The cached document and its headers, or `None`.
    ///
    /// Absent, unreadable and unparsable are all the same answer on purpose: a half-written or
    /// hand-edited cache must degrade to "no cache", which costs one `GET`, rather than to an
    /// error that stops an install the network could have served.
    pub(crate) async fn load(&self) -> Option<(RegistryDocument, CacheMeta)> {
        let body = tokio::fs::read(self.dir.join(CACHE_BODY)).await.ok()?;
        let meta = tokio::fs::read(self.dir.join(CACHE_META)).await.ok()?;
        let document = serde_json::from_slice(&body).ok()?;
        let meta = serde_json::from_slice(&meta).ok()?;
        Some((document, meta))
    }

    /// Writes both files, each through a uniquely named temporary and a rename.
    ///
    /// Mirrors `htui_store::identity::store`: a second process reading the cache concurrently
    /// sees one whole document or the previous one, never a truncated file.
    ///
    /// # Errors
    ///
    /// The `io::Error` of the first step that failed. Every caller logs it and carries on
    /// (hazard H-14): the document is already in memory, and a box that cannot write its cache
    /// can still install.
    pub(crate) async fn store(&self, body: &[u8], meta: &CacheMeta) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        self.write_atomically(CACHE_BODY, body).await?;
        self.store_meta(meta).await
    }

    /// The headers alone, for the `304` case where the body did not move but the freshness did.
    ///
    /// # Errors
    ///
    /// As [`store`](Self::store).
    pub(crate) async fn store_meta(&self, meta: &CacheMeta) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        let text = serde_json::to_vec(meta).map_err(std::io::Error::other)?;
        self.write_atomically(CACHE_META, &text).await
    }

    /// One file, tmp-then-rename.
    async fn write_atomically(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        let tmp = self.dir.join(format!("{name}.{}.tmp", tmp_suffix()));
        tokio::fs::write(&tmp, bytes).await?;
        if let Err(error) = tokio::fs::rename(&tmp, self.dir.join(name)).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
        Ok(())
    }
}

/// `<pid>-<micros>`: unique per process per microsecond, and no new dependency for it.
///
/// `htui_store::identity::store` reaches for `Uuid::now_v7()` because `uuid` is already in that
/// crate's graph; it is not in this one, and a temporary file name does not justify adding it.
fn tmp_suffix() -> String {
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_micros())
        .unwrap_or_default();
    format!("{}-{micros}", std::process::id())
}

/// Why a registry read produced no document.
///
/// Two cases and not one, because the pre-flight renders them differently: a malformed document
/// is a bug in the registry that manual steps cannot work around, while a network failure is
/// exactly what D20's derived instructions exist for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegistryError {
    /// The document could not be fetched at all and nothing was cached.
    Network(String),
    /// The document was fetched and does not parse.
    Malformed(String),
}

/// The registry read, in plan D12's order.
///
/// 1. Within the cached `max-age`, the cache answers and **no request is issued** — the pre-flight
///    of a second `i` in the same five minutes touches the network not at all.
/// 2. Past it, a conditional `GET`. A `304` keeps the cached body and only moves `fetched_at`,
///    which is the cheap case the CDN's `ETag` exists for.
/// 3. A network failure with a cached copy plans from the cache and reports its age, so the user
///    is told the document is old rather than told nothing.
/// 4. A network failure with no cache is [`RegistryError::Network`]; the caller turns it into
///    `PlanError::Network` with the manual steps derived from the row (plan D20).
///
/// A cache write that fails is a `warn!` and never an error (hazard H-14): the first `i` on a box
/// whose install root does not exist yet must not fail because a cache directory could not be
/// created.
///
/// # Errors
///
/// [`RegistryError`] per the two cases above.
pub(crate) async fn read_registry(
    http: &HttpClient,
    config: &InstallConfig,
    root: &Path,
    now: DateTime<Utc>,
) -> Result<(RegistryDocument, RegistrySource), RegistryError> {
    let cache = RegistryCache::new(root);
    let cached = cache.load().await;

    if let Some((document, meta)) = &cached
        && age_of(meta.fetched_at, now) < Duration::from_secs(meta.max_age_secs)
    {
        return Ok((document.clone(), RegistrySource::Fresh));
    }

    let url = registry_url(&config.registry_base);
    let etag = cached.as_ref().and_then(|(_, meta)| meta.etag.clone());
    match http.get_registry(&url, etag.as_deref()).await {
        Ok(RegistryFetch::NotModified) => {
            let Some((document, meta)) = cached else {
                return Err(RegistryError::Malformed(format!(
                    "{url} answered 304 and this box has nothing cached"
                )));
            };
            let refreshed = CacheMeta {
                fetched_at: now,
                ..meta
            };
            if let Err(error) = cache.store_meta(&refreshed).await {
                warn!(%error, "the registry cache headers could not be refreshed");
            }
            Ok((document, RegistrySource::Fresh))
        }
        Ok(RegistryFetch::Body {
            bytes,
            etag,
            max_age,
        }) => {
            let document: RegistryDocument = serde_json::from_slice(&bytes)
                .map_err(|error| RegistryError::Malformed(error.to_string()))?;
            let meta = CacheMeta {
                etag,
                fetched_at: now,
                max_age_secs: max_age.unwrap_or(REGISTRY_DEFAULT_MAX_AGE).as_secs(),
            };
            if let Err(error) = cache.store(&bytes, &meta).await {
                warn!(%error, "the registry document could not be cached");
            }
            Ok((document, RegistrySource::Fresh))
        }
        Err(error) => match cached {
            Some((document, meta)) => Ok((
                document,
                RegistrySource::Cached {
                    age: age_of(meta.fetched_at, now),
                },
            )),
            None => Err(RegistryError::Network(error.message)),
        },
    }
}

/// `<base>/registry.json`, tolerating a base that does or does not end in a slash.
pub(crate) fn registry_url(base: &str) -> String {
    format!("{}/registry.json", base.trim_end_matches('/'))
}

/// How old a cached copy is, clamped at zero for a clock that went backwards.
fn age_of(fetched_at: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    (now - fetched_at).to_std().unwrap_or_default()
}
