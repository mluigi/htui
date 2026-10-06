//! MOD-10 D11: one walk's secrets: the environment its sessions receive and the scrubber that
//! guards every byte it persists, as **one object**, so the two cannot drift.

use std::collections::BTreeMap;
use std::sync::Arc;

use htui_core::model::{Project, ProjectId};
use htui_core::scrub::{MinimalScrubber, Scrubber, Unmasked};
use htui_core::secret::{ResolvedSecrets, SecretError, SecretFuture, SecretSource};
use serde_json::Value;

/// [`RunSecrets::prepare`]'s [`SecretError::Config`] sentence for a walk asked about a second
/// project (MOD-10 D12's single-project guard). It never fires legitimately: one walk is one run,
/// and one run is one project.
pub const FOREIGN_PROJECT: &str = "this walk's secrets were resolved for another project";

/// One walk's secrets (MOD-10 D11). Owned by `htui-worker`'s `Kit` (one per task) and lent to
/// the engine as both `EngineParts.scrubber` and `EngineParts.secrets`.
///
/// Resolves at most once (`tokio::sync::OnceCell`), for the first project it is asked about, on
/// the first live path of the walk (MOD-10 M3 blueprint A-1) or before `AcceptArtifact`'s verify
/// (A-11). Until then, after a refusal, and
/// for a provider-less project it scrubs with the pattern rules only, exactly as the empty
/// `MinimalScrubber` it replaces. Dropping it zeroizes the resolved map and the scrubber's list.
pub struct RunSecrets {
    source: Option<Arc<dyn SecretSource>>,
    /// The pattern-only scrubber: `MinimalScrubber::new([])`.
    patterns: MinimalScrubber,
    /// Boxed so the struct stays small in every future that holds it (blueprint H-1).
    slot: tokio::sync::OnceCell<Box<Bound>>,
}

/// What the slot holds: the project it was resolved for (D12's single-project guard) and the
/// outcome, a refusal included (D12: "the failure is cached").
struct Bound {
    project: ProjectId,
    outcome: Result<Resolved, SecretError>,
}

/// A successful resolution: the map (zeroized on drop, M2) and the scrubber built from it
/// (zeroized on drop, MOD-10 D17).
struct Resolved {
    secrets: ResolvedSecrets,
    scrubber: MinimalScrubber,
}

impl RunSecrets {
    /// A walk's secrets over `source` (`None`: a process without one; a provider project is then
    /// refused with `Config`). Builds nothing and reads nothing.
    #[must_use]
    pub fn new(source: Option<Arc<dyn SecretSource>>) -> Self {
        Self {
            source,
            patterns: MinimalScrubber::new([]),
            slot: tokio::sync::OnceCell::new(),
        }
    }

    /// `new(None)`: for engines that resolve nothing (tests, tools).
    #[must_use]
    pub fn none() -> Self {
        Self::new(None)
    }

    /// MOD-10 D12: makes this walk's secrets ready for `project`.
    ///
    /// 1. The slot is already bound: to another project → `Config(FOREIGN_PROJECT)` (fail
    ///    closed, provider-less or not); to this one → its cached outcome, no second call.
    /// 2. `project.secret_provider` is `None` → `Ok(())`, the slot stays unbound, the source is
    ///    never called.
    /// 3. Otherwise the slot's `get_or_init` runs [`htui_core::secret::resolve_project`] once for
    ///    every concurrent caller; the bound project is compared again after it (two callers
    ///    racing with two projects); the outcome is answered (a refusal cloned).
    ///
    /// On success the short keys of `from_resolved` are logged once at `warn` by **name**
    /// (`project`, `keys`), never by value (D17).
    ///
    /// # Errors
    /// The cached refusal: a column fault, no source, the source's or the provider's error, a
    /// reserved key ([`SecretError`]), or [`FOREIGN_PROJECT`].
    pub fn prepare<'a>(&'a self, project: &'a Project) -> SecretFuture<'a, ()> {
        Box::pin(async move {
            if let Some(bound) = self.slot.get() {
                return answer(bound, project.id);
            }
            if project.secret_provider.is_none() {
                return Ok(());
            }
            let bound = self
                .slot
                .get_or_init(|| async {
                    let outcome = htui_core::secret::resolve_project(
                        self.source.as_deref(),
                        project,
                    )
                    .await
                    .map(|resolved| {
                        // `resolve_project` answers `None` only for a provider-less project,
                        // which returned above; an empty map is the safe reading anyway.
                        let secrets =
                            resolved.unwrap_or_else(|| ResolvedSecrets::new(BTreeMap::new()));
                        let (scrubber, short) = MinimalScrubber::from_resolved(secrets.as_map());
                        if !short.is_empty() {
                            tracing::warn!(
                                project = %project.id,
                                keys = ?short,
                                "these secrets are shorter than the masking floor: injected, \
                                 not masked"
                            );
                        }
                        Resolved { secrets, scrubber }
                    });
                    Box::new(Bound {
                        project: project.id,
                        outcome,
                    })
                })
                .await;
            answer(bound, project.id)
        })
    }

    /// MOD-10 D12: [`Self::prepare`], then the env a session of `project` receives: the
    /// resolved map cloned, or empty for a provider-less project. Nothing else ever goes in
    /// (R-SEC-2).
    ///
    /// # Errors
    /// As [`Self::prepare`].
    pub fn env_for<'a>(
        &'a self,
        project: &'a Project,
    ) -> SecretFuture<'a, BTreeMap<String, String>> {
        Box::pin(async move {
            self.prepare(project).await?;
            Ok(match self.slot.get().map(|bound| &bound.outcome) {
                Some(Ok(resolved)) => resolved.secrets.as_map().clone(),
                Some(Err(_)) | None => BTreeMap::new(),
            })
        })
    }

    /// The scrubber in force: the resolved one once bound `Ok`, else `patterns`.
    fn active(&self) -> &MinimalScrubber {
        match self.slot.get().map(|bound| &bound.outcome) {
            Some(Ok(resolved)) => &resolved.scrubber,
            Some(Err(_)) | None => &self.patterns,
        }
    }
}

/// The bound slot's answer for `project`: the foreign-project refusal, or its cached outcome.
fn answer(bound: &Bound, project: ProjectId) -> Result<(), SecretError> {
    if bound.project != project {
        return Err(SecretError::Config(FOREIGN_PROJECT.to_owned()));
    }
    match &bound.outcome {
        Ok(_) => Ok(()),
        Err(cause) => Err(cause.clone()),
    }
}

impl Scrubber for RunSecrets {
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked> {
        self.active().scrub(value)
    }

    /// MOD-10 D18: delegates, so a long resolved secret widens the recorder's seam.
    fn hold_back(&self) -> usize {
        self.active().hold_back()
    }
}

impl core::fmt::Debug for RunSecrets {
    /// `RunSecrets { source: true, state: "unresolved" | "resolved" | "refused", keys: 3 }`: no
    /// value and no key name.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (state, keys) = match self.slot.get().map(|bound| &bound.outcome) {
            None => ("unresolved", 0),
            Some(Ok(resolved)) => ("resolved", resolved.secrets.len()),
            Some(Err(_)) => ("refused", 0),
        };
        f.debug_struct("RunSecrets")
            .field("source", &self.source.is_some())
            .field("state", &state)
            .field("keys", &keys)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use htui_core::model::{Project, ProjectId, UserId};
    use htui_core::scrub::{PATTERN_HOLD_BACK, Scrubber as _};
    use htui_core::secret::fake::{FakeSecretProvider, FakeSecretSource};
    use htui_core::secret::{INFISICAL, NO_SECRET_SOURCE, PROVIDER_WITHOUT_SCOPE, SecretError};
    use serde_json::{Value, json};

    use super::{FOREIGN_PROJECT, RunSecrets};

    const SCOPE: &str = r#"{"project_id":"p1","environment":"dev","path":"/"}"#;

    /// A process's secret source, or none.
    type Source = Option<Arc<dyn htui_core::secret::SecretSource>>;
    /// Long and not pattern-shaped, so a refusal can never stand in for a mask.
    const VALUE: &str = "zq7-resolved-value-0123456789";
    /// An AWS access key id the pattern rules refuse.
    const AWS: &str = "AKIAIOSFODNN7EXAMPLE";

    fn project(provider: Option<&str>, scope: Option<&str>) -> Project {
        let now = chrono::Utc::now();
        Project {
            id: ProjectId::new(),
            slug: "p".to_owned(),
            name: "P".to_owned(),
            description: String::new(),
            secret_provider: provider.map(str::to_owned),
            secret_scope: scope.map(str::to_owned),
            settings: json!({}),
            created_by: UserId::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn infisical() -> Project {
        project(Some(INFISICAL), Some(SCOPE))
    }

    fn resolving(pairs: &[(&str, &str)]) -> (Arc<FakeSecretProvider>, Arc<FakeSecretSource>) {
        let provider = Arc::new(FakeSecretProvider::resolving(pairs));
        let source = Arc::new(FakeSecretSource::new(provider.clone()));
        (provider, source)
    }

    fn secrets_over(source: &Arc<FakeSecretSource>) -> RunSecrets {
        RunSecrets::new(Some(source.clone()))
    }

    fn scrubbed(secrets: &RunSecrets, text: &str) -> Result<String, String> {
        let mut value = Value::String(text.to_owned());
        secrets
            .scrub(&mut value)
            .map_err(|unmasked| unmasked.rule.to_owned())?;
        Ok(value.as_str().expect("a string stays a string").to_owned())
    }

    #[tokio::test]
    async fn a_provider_less_project_gets_nothing_and_never_touches_the_source() {
        let (provider, source) = resolving(&[("API_KEY", VALUE)]);
        let secrets = secrets_over(&source);
        let plain = project(None, Some("not even json"));

        let env = secrets.env_for(&plain).await.expect("provider-less is Ok");
        assert!(env.is_empty());
        assert_eq!(source.calls(), 0);
        assert_eq!(provider.resolves(), 0);
        assert!(format!("{secrets:?}").contains("unresolved"));

        // The slot stayed unbound, so a provider project (a test bug, H-12, but the slot's
        // reading) still resolves.
        let other = infisical();
        let env = secrets.env_for(&other).await.expect("resolves");
        assert_eq!(env.get("API_KEY").map(String::as_str), Some(VALUE));
    }

    #[tokio::test]
    async fn a_provider_project_resolves_once_for_every_caller() {
        let (provider, source) = resolving(&[("API_KEY", VALUE), ("OTHER", "another-value-1234")]);
        let secrets = secrets_over(&source);
        let project = infisical();

        let concurrent = futures::future::join_all((0..3).map(|_| secrets.env_for(&project))).await;
        let mut envs: Vec<BTreeMap<String, String>> = concurrent
            .into_iter()
            .map(|env| env.expect("resolves"))
            .collect();
        for _ in 0..3 {
            envs.push(secrets.env_for(&project).await.expect("cached"));
        }
        assert_eq!(source.calls(), 1);
        assert_eq!(provider.resolves(), 1);
        assert!(envs.windows(2).all(|pair| pair[0] == pair[1]));
    }

    /// R-SEC-2: the env is the resolved map, key for key, and nothing else.
    #[tokio::test]
    async fn the_env_is_the_resolved_map_and_nothing_else() {
        let (_provider, source) =
            resolving(&[("API_KEY", VALUE), ("DB_PASSWORD", "pw-0123456789")]);
        let secrets = secrets_over(&source);
        let env = secrets.env_for(&infisical()).await.expect("resolves");
        assert_eq!(
            env,
            BTreeMap::from([
                ("API_KEY".to_owned(), VALUE.to_owned()),
                ("DB_PASSWORD".to_owned(), "pw-0123456789".to_owned()),
            ])
        );
    }

    #[tokio::test]
    async fn before_resolution_only_the_pattern_rules_apply() {
        let (_provider, source) = resolving(&[("API_KEY", VALUE)]);
        let secrets = secrets_over(&source);

        assert_eq!(scrubbed(&secrets, VALUE), Ok(VALUE.to_owned()));
        assert_eq!(
            scrubbed(&secrets, &format!("key {AWS}")),
            Err("aws_access_key_id".to_owned())
        );
        assert_eq!(secrets.hold_back(), PATTERN_HOLD_BACK - 1);
    }

    #[tokio::test]
    async fn after_resolution_the_values_are_masked_and_the_seam_widens() {
        let long = "y".repeat(200);
        let (_provider, source) = resolving(&[("API_KEY", VALUE), ("LONG", &long)]);
        let secrets = secrets_over(&source);
        secrets.prepare(&infisical()).await.expect("resolves");

        let masked = scrubbed(&secrets, &format!("echo {VALUE} done")).expect("masked");
        assert!(!masked.contains(VALUE), "{masked}");
        assert!(masked.contains("[REDACTED]"), "{masked}");
        // The pattern rules still apply after resolution.
        assert_eq!(
            scrubbed(&secrets, &format!("key {AWS}")),
            Err("aws_access_key_id".to_owned())
        );
        // MOD-10 D18 (blueprint H-5): `RunSecrets` delegates `hold_back`, it does not answer 0.
        assert_eq!(secrets.hold_back(), 199);
    }

    #[tokio::test]
    async fn a_refusal_is_cached_and_never_retried() {
        let cause = SecretError::Unreachable {
            endpoint: "/api/v1/auth/universal-auth/login",
            cause: "connection refused".to_owned(),
        };
        let provider = Arc::new(FakeSecretProvider::failing(cause.clone()));
        let source = Arc::new(FakeSecretSource::new(provider.clone()));
        let secrets = secrets_over(&source);
        let project = infisical();

        let first = secrets.prepare(&project).await;
        let second = secrets.prepare(&project).await;
        assert_eq!(first, Err(cause.clone()));
        assert_eq!(second, Err(cause));
        assert_eq!(source.calls(), 1);
        assert_eq!(provider.resolves(), 1);
        assert_eq!(secrets.hold_back(), PATTERN_HOLD_BACK - 1);
        assert!(format!("{secrets:?}").contains("refused"));
    }

    #[tokio::test]
    async fn a_second_project_is_refused() {
        let (_provider, source) = resolving(&[("API_KEY", VALUE)]);
        let secrets = secrets_over(&source);
        secrets.prepare(&infisical()).await.expect("bound to A");

        let foreign = Err(SecretError::Config(FOREIGN_PROJECT.to_owned()));
        assert_eq!(secrets.prepare(&infisical()).await, foreign);
        assert_eq!(secrets.prepare(&project(None, None)).await, foreign);
        assert_eq!(
            secrets
                .env_for(&project(None, None))
                .await
                .map(|env| env.len()),
            Err(SecretError::Config(FOREIGN_PROJECT.to_owned()))
        );
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn each_column_or_source_fault_refuses_with_its_sentence() {
        let resolving_source = || {
            Some(Arc::new(FakeSecretSource::new(Arc::new(
                FakeSecretProvider::resolving(&[("API_KEY", VALUE)]),
            ))) as Arc<dyn htui_core::secret::SecretSource>)
        };
        let reserved = || {
            Some(Arc::new(FakeSecretSource::new(Arc::new(
                FakeSecretProvider::resolving(&[
                    ("API_KEY", VALUE),
                    ("HTUI_MCP_TOKEN", "shadowing-value-123"),
                ]),
            ))) as Arc<dyn htui_core::secret::SecretSource>)
        };
        let vault = || {
            Some(Arc::new(FakeSecretSource::new(Arc::new(
                FakeSecretProvider::resolving(&[("API_KEY", VALUE)]).with_kind("vault"),
            ))) as Arc<dyn htui_core::secret::SecretSource>)
        };
        let cases: Vec<(&str, Source, Project, String)> = vec![
            (
                "no source",
                None,
                infisical(),
                SecretError::Config(NO_SECRET_SOURCE.to_owned()).to_string(),
            ),
            (
                "unknown kind",
                resolving_source(),
                project(Some("vault"), Some(SCOPE)),
                "secret provider configuration: project.secret_provider \"vault\" is not a \
                 provider this build knows (expected \"infisical\")"
                    .to_owned(),
            ),
            (
                "no scope",
                resolving_source(),
                project(Some(INFISICAL), None),
                SecretError::Config(PROVIDER_WITHOUT_SCOPE.to_owned()).to_string(),
            ),
            (
                "bad scope",
                resolving_source(),
                project(
                    Some(INFISICAL),
                    Some(r#"{"project_id":"","environment":"dev"}"#),
                ),
                "secret provider configuration: the secret scope has an empty project_id"
                    .to_owned(),
            ),
            (
                "reserved key",
                reserved(),
                infisical(),
                SecretError::ReservedKey {
                    key: "HTUI_MCP_TOKEN".to_owned(),
                }
                .to_string(),
            ),
            (
                "kind mismatch",
                vault(),
                infisical(),
                "secret provider configuration: the secret source answered a `vault` provider \
                 for a `infisical` project"
                    .to_owned(),
            ),
        ];
        for (case, source, project, sentence) in cases {
            let secrets = RunSecrets::new(source);
            let refused = secrets.prepare(&project).await.expect_err(case);
            assert_eq!(refused.to_string(), sentence, "{case}");
            assert_eq!(
                secrets.env_for(&project).await,
                Err(refused),
                "{case}: cached"
            );
            assert_eq!(scrubbed(&secrets, VALUE), Ok(VALUE.to_owned()), "{case}");
        }
    }

    #[tokio::test]
    async fn short_values_are_injected_and_not_masked() {
        let (_provider, source) = resolving(&[("PIN", "123"), ("API_KEY", VALUE)]);
        let secrets = secrets_over(&source);
        let env = secrets.env_for(&infisical()).await.expect("resolves");
        assert_eq!(env.get("PIN").map(String::as_str), Some("123"));
        assert_eq!(scrubbed(&secrets, "pin 123"), Ok("pin 123".to_owned()));
    }

    #[tokio::test]
    async fn the_debug_names_no_value_and_no_key() {
        let (_provider, source) = resolving(&[("API_KEY", VALUE)]);
        let secrets = secrets_over(&source);
        let before = format!("{secrets:?}");
        secrets.prepare(&infisical()).await.expect("resolves");
        let after = format!("{secrets:?}");
        assert_eq!(
            after,
            r#"RunSecrets { source: true, state: "resolved", keys: 1 }"#
        );
        for rendered in [before, after] {
            assert!(!rendered.contains(VALUE), "{rendered}");
            assert!(!rendered.contains("API_KEY"), "{rendered}");
        }
    }

    #[test]
    fn run_secrets_is_a_send_sync_scrubber() {
        fn assert_scrubber<T: htui_core::scrub::Scrubber + Send + Sync + 'static>() {}
        assert_scrubber::<RunSecrets>();
        let secrets = RunSecrets::none();
        let _: &dyn htui_core::scrub::Scrubber = &secrets;
    }
}
