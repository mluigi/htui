//! MOD-10 D15: the process's one secret source: the OS keyring's Infisical URL and machine
//! identity, and one `InfisicalProvider` per (URL, identity).
//!
//! A walk (`htui_orch::RunSecrets`) and a chat (`agent_worker::run_chat`) ask
//! [`KeyringInfisical::provider`] once each. The keyring is read every time, so an identity or a
//! URL entered since the last walk takes effect at the next one; the provider is rebuilt only when
//! what it was built from changed, so its login latch and cool-down (MOD-10 M2 D5) survive from
//! one walk to the next.

use std::sync::Arc;

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

/// A keyring failure, as `Config`. `htui_store::secret`'s messages name slots, never values
/// (`get_machine_identity`'s half-identity sentence included).
fn keyring_unreadable(err: &htui_core::store::StoreError) -> SecretError {
    SecretError::Config(format!("the OS keyring could not be read: {err}"))
}

/// Builds a provider for a normalised URL and an identity (production: `InfisicalProvider::new`).
type Build =
    dyn Fn(&str, MachineIdentity) -> Result<Arc<dyn SecretProvider>, SecretError> + Send + Sync;

/// Reads the normalised URL and the identity, blocking (production:
/// [`KeyringInfisical::read_keyring`]).
type Read = dyn Fn() -> Result<(String, MachineIdentity), SecretError> + Send + Sync;

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
    url: String,
    client_id: String,
    secret_digest: [u8; 32],
    provider: Arc<dyn SecretProvider>,
}

impl Cached {
    /// Whether this provider was built from `url` and an identity with `client_id` and
    /// `secret_digest`.
    fn built_from(&self, url: &str, client_id: &str, secret_digest: &[u8; 32]) -> bool {
        self.url == url && self.client_id == client_id && &self.secret_digest == secret_digest
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

    /// The normalised URL and the identity the keyring holds now. Synchronous keyring I/O: run on
    /// a blocking thread (`R-NF-3`).
    fn read_keyring() -> Result<(String, MachineIdentity), SecretError> {
        let raw = htui_store::secret::get_infisical_url()
            .map_err(|err| keyring_unreadable(&err))?
            .ok_or_else(|| SecretError::Config(NO_URL.to_owned()))?;
        let url = htui_secrets::normalise_base_url(&raw)?;
        let identity = htui_store::secret::get_machine_identity()
            .map_err(|err| keyring_unreadable(&err))?
            .ok_or(SecretError::NoIdentity)?;
        Ok((url, identity))
    }

    async fn current(&self) -> Result<Arc<dyn SecretProvider>, SecretError> {
        let mut cached = self.cached.lock().await;
        let read = Arc::clone(&self.read);
        // R1 M2: the lock stays held across the read (two walks must not stack OS unlock
        // prompts), so the read is bounded. A blocking thread cannot be cancelled: one that
        // times out outlives this call until the keyring answers, and its answer is dropped.
        let (url, identity) =
            tokio::time::timeout(KEYRING_TIMEOUT, tokio::task::spawn_blocking(move || read()))
                .await
                .map_err(|_| SecretError::Config(KEYRING_SILENT.to_owned()))?
                .map_err(|_| SecretError::Config(KEYRING_UNFINISHED.to_owned()))??;
        let digest: [u8; 32] = Sha256::digest(identity.client_secret().as_bytes()).into();
        if let Some(held) = cached.as_ref()
            && held.built_from(&url, identity.client_id(), &digest)
        {
            return Ok(Arc::clone(&held.provider));
        }
        let client_id = identity.client_id().to_owned();
        let provider = (self.build)(&url, identity)?;
        *cached = Some(Cached {
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use htui_core::fixtures::ids;
    use htui_core::model::Project;
    use htui_core::secret::fake::FakeSecretProvider;
    use htui_core::secret::{INFISICAL, resolve_project};
    use htui_store::testkit::{mock_keyring, mock_keyring_broken};

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
