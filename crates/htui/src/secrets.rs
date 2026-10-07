//! MOD-10 D15: the process's one secret source: the OS keyring's Infisical URL and machine
//! identity, and one `InfisicalProvider` per (URL, identity).
//!
//! A walk (`htui_orch::RunSecrets`) and a chat (`agent_worker::run_chat`) ask
//! [`KeyringInfisical::provider`] once each. The keyring is read every time, so an identity or a
//! URL entered since the last walk takes effect at the next one; the provider is rebuilt only when
//! what it was built from changed (the URL, the identity, this process's keyring-write generation,
//! or the keyring's write mark, which carries another process's Settings write, MOD-90 D1), so its
//! login latch and cool-down (MOD-10 M2 D5) survive from one walk to the next.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use htui_core::secret::{MachineIdentity, SecretError, SecretFuture, SecretProvider, SecretSource};
use sha2::{Digest as _, Sha256};

/// D15: the URL slot is empty. The provider cannot be built without one.
pub(crate) const NO_URL: &str = "no Infisical base URL is stored in the OS keyring";

/// The blocking keyring read panicked or was cancelled before it answered.
pub(crate) const KEYRING_UNFINISHED: &str = "the OS keyring read did not finish";

/// R1 M2: the blocking keyring read did not answer within [`KEYRING_TIMEOUT`] (an OS unlock
/// prompt left unanswered, a keyring daemon that hangs).
pub(crate) const KEYRING_SILENT: &str = "the OS keyring did not answer";

/// R1 M2: how long one [`KeyringInfisical::provider`] call waits for the keyring read. Long
/// enough for a human to answer an OS unlock prompt; past it the walk or chat is refused with
/// [`KEYRING_SILENT`] and the next one asks again.
pub(crate) const KEYRING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// MOD-10 M4 (blueprint A-4): how many keyring writes `Settings > Secrets` has made in this
/// process. A provider is reused only while this is unchanged, so entering the identity again —
/// even the same one — gives the next walk, chat or check a fresh provider without the old
/// one's login latch (M2 D5). In process only: the keyring's write mark carries a write to
/// another process (`htui worker`, MOD-90 D1).
static KEYRING_WRITES: AtomicU64 = AtomicU64::new(0);

/// Called by `secrets_settings` after every successful URL or identity write or clear, with
/// [`keyring_io`] still held. The generation the write made.
pub(crate) fn note_keyring_write() -> u64 {
    KEYRING_WRITES.fetch_add(1, Ordering::SeqCst) + 1
}

/// MOD-10 M4 R1 L-2: serialises this process's keyring I/O. The identity is two keyring entries,
/// written one after the other and read one after the other, so a read between the two writes of
/// a Settings identity write would pair the new client ID with the old secret (or the reverse)
/// and spend a login on it. [`KeyringInfisical::read_keyring`] reads the write mark and both
/// halves, and every Settings keyring write writes, stores a new write mark (MOD-90 D2) and calls
/// [`note_keyring_write`], under this lock. Another process (`htui worker`) is not covered: it may
/// still read half of a write, but the mark, read first and written last, never lets it miss one.
///
/// Held on blocking threads only. A read waiting on an OS unlock prompt holds it, so a Settings
/// write waits for that prompt too: the loop stall H-7 already accepts.
static KEYRING_IO: Mutex<()> = Mutex::new(());

/// [`KEYRING_IO`], taken on a blocking thread. A poisoned lock guards no data, so it is taken
/// anyway.
pub(crate) fn keyring_io() -> MutexGuard<'static, ()> {
    KEYRING_IO
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

tokio::task_local! {
    /// [`provider_with_generation`]'s slot: [`KeyringInfisical::current`] stores the generation
    /// it built or reused its provider at.
    static PROVIDER_GENERATION: Cell<u64>;
}

/// MOD-10 M4 R1 L-1: `source.provider()`, and the keyring-write generation that provider was
/// built at ([`KeyringInfisical`]'s own; for any other source, the generation when the call
/// started). The provider check reports it, so `Settings > Secrets` tells whether a landed write
/// replaced the provider the check latched from the generations rather than from reply order: the
/// check runs in a spawned task, and can wait on a walk's keyring read and build after a write
/// sent later.
pub(crate) async fn provider_with_generation(
    source: &dyn SecretSource,
) -> (u64, Result<Arc<dyn SecretProvider>, SecretError>) {
    PROVIDER_GENERATION
        .scope(Cell::new(KEYRING_WRITES.load(Ordering::SeqCst)), async {
            let provider = source.provider().await;
            (PROVIDER_GENERATION.with(Cell::get), provider)
        })
        .await
}

/// A keyring failure, as `Config`. `htui_store::secret`'s messages name slots, never values
/// (`get_machine_identity`'s half-identity sentence included).
fn keyring_unreadable(err: &htui_core::store::StoreError) -> SecretError {
    SecretError::Config(format!("the OS keyring could not be read: {err}"))
}

/// Builds a provider for a normalised URL and an identity (production: `InfisicalProvider::new`).
type Build =
    dyn Fn(&str, MachineIdentity) -> Result<Arc<dyn SecretProvider>, SecretError> + Send + Sync;

/// One keyring read (MOD-90 D1): the write mark, read first (D2), the normalised URL and the
/// identity. All three are the provider's cache key.
struct KeyringRead {
    mark: Option<String>,
    url: String,
    identity: MachineIdentity,
}

/// Reads the write mark, the normalised URL and the identity, blocking (production:
/// [`KeyringInfisical::read_keyring`]).
type Read = dyn Fn() -> Result<KeyringRead, SecretError> + Send + Sync;

/// MOD-10 D15: the keyring-backed Infisical [`SecretSource`]. One per process (the TUI shares
/// its one between the run and agent runtimes; `htui worker` has its own). Building it reads
/// nothing: the keyring is read per [`provider`](SecretSource::provider) call.
pub struct KeyringInfisical {
    build: Box<Build>,
    /// Run on a blocking thread, under [`KEYRING_TIMEOUT`].
    read: Arc<Read>,
    /// Held across the keyring read and the build, so two walks starting together build once
    /// and do not stack OS unlock prompts. [`KEYRING_TIMEOUT`] bounds how long it is held.
    cached: tokio::sync::Mutex<Option<Cached>>,
}

/// The provider and what it was built from. The client secret is kept only as a SHA-256 digest
/// to compare; the provider holds the identity itself.
struct Cached {
    /// [`KEYRING_WRITES`] as read before the keyring read this provider was built from.
    generation: u64,
    /// The write mark the provider was built under (MOD-90).
    mark: Option<String>,
    url: String,
    client_id: String,
    secret_digest: [u8; 32],
    provider: Arc<dyn SecretProvider>,
}

impl Cached {
    /// Whether this provider was built from `read`'s URL and an identity with its client ID and
    /// `secret_digest`, with no Settings keyring write since: none in this process (`generation`,
    /// blueprint A-4), none in another (`read`'s mark, MOD-90 D1).
    fn built_from(&self, generation: u64, read: &KeyringRead, secret_digest: &[u8; 32]) -> bool {
        self.generation == generation
            && self.mark == read.mark
            && self.url == read.url
            && self.client_id == read.identity.client_id()
            && &self.secret_digest == secret_digest
    }
}

impl KeyringInfisical {
    /// Production: `InfisicalProvider::new(InfisicalConfig::new(url), identity)`.
    #[must_use]
    pub fn new() -> Self {
        Self::from_parts(
            Box::new(|url, identity| {
                let provider = htui_secrets::InfisicalProvider::new(
                    htui_secrets::InfisicalConfig::new(url),
                    identity,
                )?;
                Ok(Arc::new(provider) as Arc<dyn SecretProvider>)
            }),
            Arc::new(Self::read_keyring),
        )
    }

    /// Tests: any builder (a counting one returning `FakeSecretProvider`s) over the keyring.
    #[cfg(test)]
    fn with_builder(build: Box<Build>) -> Self {
        Self::from_parts(build, Arc::new(Self::read_keyring))
    }

    /// Tests: any builder and any read (one that blocks, R1 M2).
    #[cfg(test)]
    fn with_parts(build: Box<Build>, read: Arc<Read>) -> Self {
        Self::from_parts(build, read)
    }

    fn from_parts(build: Box<Build>, read: Arc<Read>) -> Self {
        Self {
            build,
            read,
            cached: tokio::sync::Mutex::new(None),
        }
    }

    /// The write mark, the normalised URL and the identity the keyring holds now, under
    /// [`keyring_io`] (R1 L-2: never half of a Settings write). Synchronous keyring I/O: run on a
    /// blocking thread (`R-NF-3`).
    fn read_keyring() -> Result<KeyringRead, SecretError> {
        let _io = keyring_io();
        // MOD-90 D2: first. A Settings write stores it last, so a read racing another process's
        // write costs at most one extra rebuild, never a missed one. D4: unreadable refuses, as a
        // URL does.
        let mark = htui_store::secret::get_infisical_write_mark()
            .map_err(|err| keyring_unreadable(&err))?;
        let raw = htui_store::secret::get_infisical_url()
            .map_err(|err| keyring_unreadable(&err))?
            .ok_or_else(|| SecretError::Config(NO_URL.to_owned()))?;
        let url = htui_secrets::normalise_base_url(&raw)?;
        let identity = htui_store::secret::get_machine_identity()
            .map_err(|err| keyring_unreadable(&err))?
            .ok_or(SecretError::NoIdentity)?;
        Ok(KeyringRead {
            mark,
            url,
            identity,
        })
    }

    async fn current(&self) -> Result<Arc<dyn SecretProvider>, SecretError> {
        let mut cached = self.cached.lock().await;
        // Before the read (blueprint A-4): a write that lands during it forces one more rebuild
        // next time, never one too few.
        let generation = KEYRING_WRITES.load(Ordering::SeqCst);
        // R1 L-1: the provider handed out below, cached or built, is this generation's.
        let _ = PROVIDER_GENERATION.try_with(|at| at.set(generation));
        let read = Arc::clone(&self.read);
        // R1 M2: the lock stays held across the read (two walks must not stack OS unlock
        // prompts), so the read is bounded. A blocking thread cannot be cancelled: one that
        // times out outlives this call until the keyring answers, and its answer is dropped.
        let read: KeyringRead =
            tokio::time::timeout(KEYRING_TIMEOUT, tokio::task::spawn_blocking(move || read()))
                .await
                .map_err(|_| SecretError::Config(KEYRING_SILENT.to_owned()))?
                .map_err(|_| SecretError::Config(KEYRING_UNFINISHED.to_owned()))??;
        let digest: [u8; 32] = Sha256::digest(read.identity.client_secret().as_bytes()).into();
        if let Some(held) = cached.as_ref()
            && held.built_from(generation, &read, &digest)
        {
            return Ok(Arc::clone(&held.provider));
        }
        let KeyringRead {
            mark,
            url,
            identity,
        } = read;
        let client_id = identity.client_id().to_owned();
        let provider = (self.build)(&url, identity)?;
        *cached = Some(Cached {
            generation,
            mark,
            url,
            client_id,
            secret_digest: digest,
            provider: Arc::clone(&provider),
        });
        Ok(provider)
    }
}

impl Default for KeyringInfisical {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for KeyringInfisical {
    /// `KeyringInfisical { cached: true }`: no URL, no identity. While a `provider()` call holds
    /// the cache, `KeyringInfisical { busy: true }`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut out = f.debug_struct("KeyringInfisical");
        match self.cached.try_lock() {
            Ok(cached) => out.field("cached", &cached.is_some()),
            Err(_) => out.field("busy", &true),
        };
        out.finish()
    }
}

impl SecretSource for KeyringInfisical {
    fn provider(&self) -> SecretFuture<'_, Arc<dyn SecretProvider>> {
        Box::pin(self.current())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use htui_core::fixtures::ids;
    use htui_core::model::Project;
    use htui_core::secret::fake::FakeSecretProvider;
    use htui_core::secret::{INFISICAL, resolve_project};
    use htui_store::testkit::{mock_keyring, mock_keyring_broken, refuse_fake_store};

    use super::*;

    const URL: &str = "https://x.example";
    const CLIENT_ID: &str = "cid-1";
    const CLIENT_SECRET: &str = "zq7-client-secret-0123456789";
    const SCOPE_COLUMN: &str = r#"{"project_id":"p1","environment":"dev","path":"/"}"#;

    /// What a counting builder saw: how many builds, and the URLs they were for.
    #[derive(Default)]
    struct Builds {
        count: AtomicUsize,
        urls: Mutex<Vec<String>>,
    }

    impl Builds {
        fn count(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }
    }

    /// A source whose builder counts its builds and answers `provider()`'s output.
    fn counting(provider: fn() -> FakeSecretProvider) -> (KeyringInfisical, Arc<Builds>) {
        let builds = Arc::new(Builds::default());
        let seen = Arc::clone(&builds);
        let source = KeyringInfisical::with_builder(Box::new(move |url, _identity| {
            seen.count.fetch_add(1, Ordering::SeqCst);
            seen.urls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(url.to_owned());
            Ok(Arc::new(provider()) as Arc<dyn SecretProvider>)
        }));
        (source, builds)
    }

    fn resolving() -> FakeSecretProvider {
        FakeSecretProvider::resolving(&[("API_KEY", "zq7-resolved-value-0123456789")])
    }

    fn store_identity(client_id: &str, client_secret: &str) {
        htui_store::secret::set_machine_identity(&MachineIdentity::new(client_id, client_secret))
            .expect("the fake keyring stores");
    }

    fn store_url(url: &str) {
        htui_store::secret::set_infisical_url(url).expect("the fake keyring stores");
    }

    /// Stores `mark` as the write mark, as another process's Settings write does (MOD-90 D1).
    fn store_mark(mark: &str) {
        htui_store::secret::set_infisical_write_mark(mark).expect("the fake keyring stores");
    }

    fn config(err: SecretError) -> String {
        match err {
            SecretError::Config(sentence) => sentence,
            other => panic!("expected a Config refusal, got {other:?}"),
        }
    }

    async fn refusal(source: &KeyringInfisical) -> SecretError {
        match source.provider().await {
            Ok(provider) => panic!("expected a refusal, got {provider:?}"),
            Err(err) => err,
        }
    }

    fn provider_project() -> Project {
        let mut project = htui_core::fixtures::demo_data()
            .projects
            .into_iter()
            .find(|project| project.id == ids::PROJECT_HTUI)
            .expect("the demo project");
        project.secret_provider = Some(INFISICAL.to_owned());
        project.secret_scope = Some(SCOPE_COLUMN.to_owned());
        project
    }

    #[tokio::test]
    async fn no_stored_url_refuses_with_the_d15_sentence() {
        let _guard = mock_keyring().await;
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(resolving);
        assert_eq!(config(refusal(&source).await), NO_URL);
        assert_eq!(builds.count(), 0, "nothing is built without a URL");
    }

    #[tokio::test]
    async fn no_stored_identity_is_no_identity() {
        let _guard = mock_keyring().await;
        store_url(URL);
        let (source, builds) = counting(resolving);
        assert_eq!(refusal(&source).await, SecretError::NoIdentity);
        assert_eq!(builds.count(), 0);
    }

    #[tokio::test]
    async fn a_half_stored_identity_refuses_naming_the_keyring_only() {
        let _guard = mock_keyring().await;
        store_url(URL);
        // A blank half reads as absent (`htui_store::secret::read_slot`): a half identity.
        htui_store::secret::set_machine_identity(&MachineIdentity::new("   ", CLIENT_SECRET))
            .expect("the fake keyring stores");
        let (source, builds) = counting(resolving);
        let err = refusal(&source).await;
        let text = err.to_string();
        let sentence = config(err);
        assert!(
            sentence.starts_with("the OS keyring could not be read: "),
            "{sentence}"
        );
        assert!(sentence.contains("infisical-client-id"), "{sentence}");
        assert!(
            !text.contains(CLIENT_SECRET),
            "the refusal never carries the client secret"
        );
        assert_eq!(builds.count(), 0);
    }

    #[tokio::test]
    async fn a_broken_keyring_refuses_naming_the_keyring() {
        let _guard = mock_keyring_broken().await;
        let (source, builds) = counting(resolving);
        let sentence = config(refusal(&source).await);
        assert!(
            sentence.starts_with("the OS keyring could not be read: "),
            "{sentence}"
        );
        assert!(
            sentence.contains(htui_store::testkit::BROKEN_KEYRING),
            "{sentence}"
        );
        assert_eq!(builds.count(), 0);
    }

    #[tokio::test]
    async fn a_bad_stored_url_is_refused_by_normalisation() {
        let _guard = mock_keyring().await;
        store_url("http://192.168.1.10");
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(resolving);
        let err = refusal(&source).await;
        assert_eq!(
            err,
            htui_secrets::normalise_base_url("http://192.168.1.10")
                .expect_err("plain http to a LAN host is refused"),
            "the refusal is normalisation's own"
        );
        assert_eq!(builds.count(), 0);
    }

    #[tokio::test]
    async fn an_unchanged_keyring_reuses_the_provider() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(resolving);
        let first = source.provider().await.expect("a provider");
        let second = source.provider().await.expect("a provider");
        assert!(
            Arc::ptr_eq(&first, &second),
            "the same provider is handed out"
        );
        assert_eq!(builds.count(), 1);
    }

    #[tokio::test]
    async fn an_equivalent_url_spelling_reuses_the_provider() {
        let _guard = mock_keyring().await;
        store_url("https://x.example/");
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(resolving);
        let first = source.provider().await.expect("a provider");
        store_url("https://x.example");
        let second = source.provider().await.expect("a provider");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(builds.count(), 1);
        assert_eq!(
            *builds
                .urls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![URL.to_owned()],
            "the builder is handed the normalised URL"
        );
    }

    #[tokio::test]
    async fn a_new_identity_or_url_rebuilds_the_provider() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(resolving);
        let first = source.provider().await.expect("a provider");

        store_identity(CLIENT_ID, "zq7-another-secret-0123456789");
        let second = source.provider().await.expect("a provider");
        assert_eq!(builds.count(), 2, "a new client secret rebuilds");
        assert!(!Arc::ptr_eq(&first, &second));

        store_identity("cid-2", "zq7-another-secret-0123456789");
        let third = source.provider().await.expect("a provider");
        assert_eq!(builds.count(), 3, "a new client ID rebuilds");
        assert!(!Arc::ptr_eq(&second, &third));

        store_url("https://y.example");
        let fourth = source.provider().await.expect("a provider");
        assert_eq!(builds.count(), 4, "a new URL rebuilds");
        assert!(!Arc::ptr_eq(&third, &fourth));

        let fifth = source.provider().await.expect("a provider");
        assert!(Arc::ptr_eq(&fourth, &fifth), "and the new one is kept");
        assert_eq!(builds.count(), 4);
    }

    /// M2 D5 across walks: the provider that refused the login is the one the next walk asks, so
    /// it answers its latch rather than logging in again.
    #[tokio::test]
    async fn the_login_latch_survives_two_walks() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(|| {
            FakeSecretProvider::new([
                Err(SecretError::BadCredentials),
                Err(SecretError::LoginRefusedEarlier),
            ])
        });
        let project = provider_project();
        // One `resolve_project` per walk: what `RunSecrets::prepare` and a chat each run once.
        let walk_1 = resolve_project(Some(&source), &project)
            .await
            .expect_err("walk 1 refuses");
        let walk_2 = resolve_project(Some(&source), &project)
            .await
            .expect_err("walk 2 refuses");
        assert_eq!(walk_1, SecretError::BadCredentials);
        assert_eq!(walk_2, SecretError::LoginRefusedEarlier);
        assert_eq!(builds.count(), 1, "one provider served both walks");
    }

    /// M4 blueprint A-4: re-entering the **same** identity after a refused login (an
    /// `IdentityLocked`, or a fix on the server with an unchanged secret) changes none of the URL,
    /// the client ID or the secret digest; the Settings write's generation alone gives the next
    /// call a fresh provider without the latch.
    #[tokio::test]
    async fn entering_the_same_identity_again_rebuilds_a_latched_provider() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(|| {
            FakeSecretProvider::new([
                Err(SecretError::BadCredentials),
                Err(SecretError::LoginRefusedEarlier),
            ])
        });
        let first = source.provider().await.expect("a provider");

        store_identity(CLIENT_ID, CLIENT_SECRET);
        note_keyring_write();
        let second = source.provider().await.expect("a provider");

        assert_eq!(
            builds.count(),
            2,
            "the same identity, written again, rebuilds"
        );
        assert!(!Arc::ptr_eq(&first, &second));
        let third = source.provider().await.expect("a provider");
        assert!(Arc::ptr_eq(&second, &third), "and the new one is kept");
    }

    /// MOD-90 D1: another process's Settings write (`htui worker` sees the TUI's) bumps no
    /// generation here; the write mark it stored alone rebuilds the latched provider.
    #[tokio::test]
    async fn a_write_mark_from_another_process_rebuilds_a_latched_provider() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        store_mark("m-1");
        let (source, builds) = counting(|| {
            FakeSecretProvider::new([
                Err(SecretError::BadCredentials),
                Err(SecretError::LoginRefusedEarlier),
            ])
        });
        let first = source.provider().await.expect("a provider");

        // The other process's write: the same identity again, then a new mark. No
        // `note_keyring_write`: that generation is the other process's.
        store_identity(CLIENT_ID, CLIENT_SECRET);
        store_mark("m-2");
        let second = source.provider().await.expect("a provider");
        let third = source.provider().await.expect("a provider");

        assert_eq!(builds.count(), 2, "the new mark rebuilds");
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&second, &third), "and the new one is kept");
    }

    /// MOD-90 D1: nothing rebuilds unless a write happened, so a refused login is never retried
    /// on its own.
    #[tokio::test]
    async fn an_unchanged_mark_and_identity_keep_the_latched_provider() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        store_mark("m-1");
        let (source, builds) = counting(|| {
            FakeSecretProvider::new([
                Err(SecretError::BadCredentials),
                Err(SecretError::LoginRefusedEarlier),
            ])
        });
        let project = provider_project();
        let walk_1 = resolve_project(Some(&source), &project)
            .await
            .expect_err("walk 1 refuses");
        let walk_2 = resolve_project(Some(&source), &project)
            .await
            .expect_err("walk 2 refuses");
        assert_eq!(walk_1, SecretError::BadCredentials);
        assert_eq!(walk_2, SecretError::LoginRefusedEarlier);
        assert_eq!(builds.count(), 1, "one provider served both walks");
    }

    /// MOD-90 D4: a keyring written before MOD-90 has no mark; `None` is a stable key, and a
    /// first mark is a change.
    #[tokio::test]
    async fn a_missing_mark_is_a_valid_key() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(resolving);
        let first = source.provider().await.expect("a provider");
        let second = source.provider().await.expect("a provider");
        assert_eq!(builds.count(), 1, "no mark, twice, reuses");
        assert!(Arc::ptr_eq(&first, &second));

        store_mark("m-1");
        source.provider().await.expect("a provider");
        assert_eq!(builds.count(), 2, "a first mark rebuilds");
    }

    /// MOD-90 D2, D4 (blueprint A-5): the mark is read first, and an unreadable one refuses the
    /// walk as an unreadable URL does. Only a mark read that comes first names the mark's slot.
    #[tokio::test]
    async fn a_broken_keyring_refuses_at_the_mark_read_first() {
        let _guard = mock_keyring_broken().await;
        let (source, builds) = counting(resolving);
        let sentence = config(refusal(&source).await);
        assert!(
            sentence.starts_with("the OS keyring could not be read: "),
            "{sentence}"
        );
        assert!(sentence.contains("infisical-write-mark"), "{sentence}");
        assert_eq!(builds.count(), 0);
    }

    /// MOD-90 D3: a mark the keyring refuses to store costs only the other processes; this one
    /// still rebuilds through its generation.
    #[tokio::test]
    async fn a_refused_mark_write_still_rebuilds_in_this_process() {
        use crate::secrets_settings::{IdentityEntry, Redacted};
        use crate::store_worker::StoreRequest;

        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (_root, backend) = offline().await;
        refuse_fake_store(htui_store::secret::INFISICAL_WRITE_MARK_USER);
        let (source, builds) = counting(|| {
            FakeSecretProvider::new([
                Err(SecretError::BadCredentials),
                Err(SecretError::LoginRefusedEarlier),
            ])
        });
        let first = source.provider().await.expect("a provider");

        let same_identity = IdentityEntry::new(
            CLIENT_ID.to_owned(),
            Redacted::new(CLIENT_SECRET.to_owned()),
        );
        serve_write(&backend, StoreRequest::SetMachineIdentity(same_identity)).await;
        let second = source.provider().await.expect("a provider");

        assert_eq!(builds.count(), 2, "the generation rebuilds");
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(
            htui_store::secret::get_infisical_write_mark().expect("the fake keyring answers"),
            None,
            "the refused mark was not stored"
        );
    }

    /// An offline backend over a throwaway mirror: not `Memory`, so `secrets_settings::serve`
    /// writes the keyring rather than refusing a demo session.
    async fn offline() -> (tempfile::TempDir, htui_store::Backend) {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "secrets-generation", 1)
            .await
            .expect("open a throwaway mirror");
        (root, htui_store::Backend::Offline { cache, since: None })
    }

    /// Serves `request` through the Settings > Secrets worker arm and expects its own answer.
    async fn serve_write(
        backend: &htui_store::Backend,
        request: crate::store_worker::StoreRequest,
    ) {
        let reply = crate::secrets_settings::serve(backend, &request)
            .await
            .expect("the write is served");
        assert!(
            matches!(
                reply,
                crate::store_worker::StoreReply::SecretsWritten { request: named, .. }
                    if named == request.name()
            ),
            "the write lands: {reply:?}"
        );
    }

    /// M4 blueprint A-4 end to end: the Settings writes themselves, served through
    /// `secrets_settings::serve`, give the next `provider()` a fresh provider even when they store
    /// the very URL and identity the latched one was built from; a refused write does not.
    #[tokio::test]
    async fn a_settings_keyring_write_rebuilds_a_latched_provider() {
        use crate::secrets_settings::{IdentityEntry, Redacted};
        use crate::store_worker::StoreRequest;

        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (_root, backend) = offline().await;
        let (source, builds) = counting(|| {
            FakeSecretProvider::new([
                Err(SecretError::BadCredentials),
                Err(SecretError::LoginRefusedEarlier),
            ])
        });
        let first = source.provider().await.expect("a provider");

        let same_identity = || {
            IdentityEntry::new(
                CLIENT_ID.to_owned(),
                Redacted::new(CLIENT_SECRET.to_owned()),
            )
        };
        serve_write(&backend, StoreRequest::SetMachineIdentity(same_identity())).await;
        let second = source.provider().await.expect("a provider");
        assert_eq!(
            builds.count(),
            2,
            "the same identity, saved again, rebuilds"
        );
        assert!(!Arc::ptr_eq(&first, &second));

        serve_write(&backend, StoreRequest::SetInfisicalUrl(URL.to_owned())).await;
        let third = source.provider().await.expect("a provider");
        assert_eq!(builds.count(), 3, "the same URL, saved again, rebuilds");
        assert!(!Arc::ptr_eq(&second, &third));

        let blank = IdentityEntry::new(CLIENT_ID.to_owned(), Redacted::new(String::new()));
        let refused =
            crate::secrets_settings::serve(&backend, &StoreRequest::SetMachineIdentity(blank))
                .await
                .expect("a refusal is a reply");
        assert!(
            matches!(refused, crate::store_worker::StoreReply::Failed { .. }),
            "a blank half is refused: {refused:?}"
        );
        let fourth = source.provider().await.expect("a provider");
        assert_eq!(builds.count(), 3, "a refused write keeps the provider");
        assert!(Arc::ptr_eq(&third, &fourth));
    }

    /// R1 M2: a keyring read that never answers (an OS unlock prompt nobody answers) refuses
    /// the walk after [`KEYRING_TIMEOUT`] and gives the lock back; the blocking read itself
    /// cannot be cancelled and is released here only so the test does not leak its thread.
    #[tokio::test(start_paused = true)]
    async fn a_keyring_that_never_answers_is_refused_after_the_timeout() {
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (release, held) = std::sync::mpsc::channel::<()>();
        let held = Mutex::new(held);
        let seen = Arc::clone(&started);
        let source = Arc::new(KeyringInfisical::with_parts(
            Box::new(|_url, _identity| Ok(Arc::new(resolving()) as Arc<dyn SecretProvider>)),
            Arc::new(move || {
                seen.store(true, Ordering::SeqCst);
                let _ = held
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .recv();
                Err(SecretError::NoIdentity)
            }),
        ));
        let asking = Arc::clone(&source);
        let call = tokio::spawn(async move { asking.provider().await.map(|_| ()) });
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }

        tokio::time::advance(KEYRING_TIMEOUT - std::time::Duration::from_secs(1)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            !call.is_finished(),
            "still waiting one second before the timeout"
        );
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        let refused = call.await.expect("the call does not panic");

        assert_eq!(refused, Err(SecretError::Config(KEYRING_SILENT.to_owned())));
        assert_eq!(
            format!("{source:?}"),
            "KeyringInfisical { cached: false }",
            "the lock was given back"
        );
        release
            .send(())
            .expect("the blocking read is still waiting");
    }

    /// Takes [`keyring_io`] on a thread of its own, as a Settings write in progress does, until
    /// the returned sender is sent to or dropped.
    fn hold_keyring_io() -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
        let (taken, is_taken) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            let _io = keyring_io();
            taken.send(()).expect("the test waits for the lock");
            let _ = released.recv();
        });
        is_taken.recv().expect("the holder takes the lock");
        (release, holder)
    }

    /// Whether `task` is still running after a while of real time.
    async fn still_waiting<T>(task: &tokio::task::JoinHandle<T>) -> bool {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        !task.is_finished()
    }

    /// R1 L-2: the keyring read waits for a Settings write in progress, so it never pairs one
    /// half of the identity it wrote with the other half it replaced.
    #[tokio::test]
    async fn a_keyring_read_waits_for_a_settings_write_in_progress() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, builds) = counting(resolving);
        let source = Arc::new(source);
        let (release, holder) = hold_keyring_io();
        let asking = Arc::clone(&source);
        let call = tokio::spawn(async move { asking.provider().await.map(|_| ()) });
        assert!(still_waiting(&call).await, "the read waits for the write");
        assert_eq!(builds.count(), 0);

        release.send(()).expect("the holder waits");
        holder.join().expect("the holder does not panic");
        call.await
            .expect("the call does not panic")
            .expect("a provider");
        assert_eq!(builds.count(), 1);
    }

    /// What the keyring holds now, read past [`keyring_io`]: the URL, whether an identity, and
    /// the write mark (MOD-90 D2: written under the lock too).
    fn keyring_now() -> (Option<String>, bool, Option<String>) {
        (
            htui_store::secret::get_infisical_url().expect("the fake keyring answers"),
            htui_store::secret::get_machine_identity()
                .expect("the fake keyring answers")
                .is_some(),
            htui_store::secret::get_infisical_write_mark().expect("the fake keyring answers"),
        )
    }

    /// R1 L-2: every Settings keyring write, its write mark (MOD-90 D2) and its generation wait
    /// for a keyring read in progress.
    #[tokio::test]
    async fn a_settings_keyring_write_waits_for_a_keyring_read_in_progress() {
        use crate::secrets_settings::{IdentityEntry, Redacted};
        use crate::store_worker::StoreRequest;

        let _guard = mock_keyring().await;
        let (_root, backend) = offline().await;
        let backend = Arc::new(backend);
        let identity = || {
            IdentityEntry::new(
                CLIENT_ID.to_owned(),
                Redacted::new(CLIENT_SECRET.to_owned()),
            )
        };
        for request in [
            StoreRequest::SetInfisicalUrl(URL.to_owned()),
            StoreRequest::ClearInfisicalUrl,
            StoreRequest::SetMachineIdentity(identity()),
            StoreRequest::ClearMachineIdentity,
        ] {
            let name = request.name();
            let before = (keyring_now(), KEYRING_WRITES.load(Ordering::SeqCst));
            let (release, holder) = hold_keyring_io();
            let serving = Arc::clone(&backend);
            let write = tokio::spawn(async move { serve_write(&serving, request).await });
            assert!(still_waiting(&write).await, "{name} waits for the read");
            assert_eq!(
                (keyring_now(), KEYRING_WRITES.load(Ordering::SeqCst)),
                before,
                "{name} has written nothing yet"
            );
            release.send(()).expect("the holder waits");
            holder.join().expect("the holder does not panic");
            write.await.expect("the write lands");
            assert_ne!(keyring_now(), before.0, "{name} wrote");
        }
    }

    /// R1 L-1: a provider check that waits on a walk's keyring read, while a Settings write
    /// lands, builds after that write; it learns the write's generation, the one its provider was
    /// built at, not the one when it started.
    #[tokio::test]
    async fn the_check_learns_the_generation_its_provider_was_built_at() {
        // No keyring here: the guard only keeps the other generation bumps out.
        let _guard = mock_keyring().await;
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (release, held) = std::sync::mpsc::channel::<()>();
        let held = Mutex::new(Some(held));
        let seen = Arc::clone(&started);
        let builds = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&builds);
        let source = Arc::new(KeyringInfisical::with_parts(
            Box::new(move |_url, _identity| {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(resolving()) as Arc<dyn SecretProvider>)
            }),
            Arc::new(move || {
                seen.store(true, Ordering::SeqCst);
                // The walk's read waits for the test; the check's answers at once.
                let first = held
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let Some(first) = first {
                    let _ = first.recv();
                }
                Ok(KeyringRead {
                    mark: None,
                    url: URL.to_owned(),
                    identity: MachineIdentity::new(CLIENT_ID, CLIENT_SECRET),
                })
            }),
        ));
        let walking = Arc::clone(&source);
        let walk = tokio::spawn(async move { walking.provider().await.map(|_| ()) });
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        let checking = Arc::clone(&source);
        let check = tokio::spawn(async move {
            let (generation, provider) = provider_with_generation(checking.as_ref()).await;
            (generation, provider.map(|_| ()))
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(!check.is_finished(), "the check waits on the walk's read");

        let written = note_keyring_write();
        release.send(()).expect("the walk's read waits");
        walk.await
            .expect("the walk does not panic")
            .expect("a provider");
        let (generation, built) = check.await.expect("the check does not panic");
        built.expect("a provider");
        assert_eq!(
            generation, written,
            "built after the write, at its generation"
        );
        assert_eq!(builds.load(Ordering::SeqCst), 2, "the check rebuilt");
    }

    #[tokio::test]
    async fn the_source_never_prints_a_secret() {
        let _guard = mock_keyring().await;
        store_url(URL);
        store_identity(CLIENT_ID, CLIENT_SECRET);
        let (source, _builds) = counting(resolving);
        assert_eq!(format!("{source:?}"), "KeyringInfisical { cached: false }");
        source.provider().await.expect("a provider");
        let shown = format!("{source:?}");
        assert_eq!(shown, "KeyringInfisical { cached: true }");
        for value in [CLIENT_SECRET, CLIENT_ID, "x.example"] {
            assert!(!shown.contains(value), "{shown}");
        }
    }
}
