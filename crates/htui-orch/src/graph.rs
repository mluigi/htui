//! Resolution: a live `step_graph` becomes the typed `GraphSnapshot` a run is decided by.
//!
//! ANA-2 §4.1's field chains (`docs/ANA-2.md:278-288`), the `topology` digest the snapshot row
//! reserves for this milestone (`crates/htui-core/src/model/run.rs:453`), the scope resolution
//! that refuses an empty scope for an item with a primary repo (plan D14), and the override clone
//! that is deep over `step_graph_phase` and never over `skill_binding` (`docs/ANA-2.md:290-295`).
//!
//! Resolution is the **only** place that reads `step_graph_phase`. Once a run exists, the walk
//! reads its own `graph_snapshot` and nothing else (invariant 2, `docs/ANA-2.md:109-113`), which
//! is what makes an edit to a phase mid-run a no-op for that run.

use std::collections::BTreeMap;

use htui_core::model::{
    Agent, AgentId, GraphSnapshot, Isolation, Item, ItemId, ItemPatch, NewStepGraph, PhaseAgent,
    PhaseId, Project, ProjectId, ProjectSettings, PromptTemplate, Repo, RepoId, ResolvedGraph,
    RunMode, SnapshotCandidate, SnapshotGraph, SnapshotJudge, SnapshotPhase, SnapshotSettings,
    SnapshotTemplate, StepGraph, StepGraphId, StepGraphPhase,
};
use htui_core::store::{ReadStore, Result, StoreError, UpdateOutcome, WriteStore};
use serde_json::Value;

/// `deadline_seconds`' built-in default: two hours (`docs/ANA-2.md:286`).
const DEFAULT_DEADLINE_SECONDS: u32 = 7200;
/// `app_setting.max_fan_out`'s built-in default (ANA-2 §10, §5.1's example).
const DEFAULT_MAX_FAN_OUT: u32 = 4;
/// `app_setting.max_agents_per_run`'s built-in default (ANA-2 §10, §5.1's example).
const DEFAULT_MAX_AGENTS_PER_RUN: u32 = 6;

/// What `htui-orch` needs from a store to build a [`GraphSnapshot`] and that no `ReadStore`
/// method answers (plan D19).
///
/// The eleven orchestration reads milestone 1 shipped are **inherent** on `MemStore`
/// (`crates/htui-core/src/store/mem.rs:445-644`) and on `htui-store`'s `Backend`, on no trait,
/// because the tables behind them are not mirrored. A crate generic over `S: ReadStore +
/// WriteStore` therefore cannot reach `resolve_graph`, `phase_agents`, `prompt_template` or the
/// agent registry at all, and three `GraphSnapshot` fields — `SnapshotPhase::candidates`,
/// `SnapshotTemplate::version` and the denormalised `SnapshotCandidate::agent_name` — have no
/// generic source whatsoever. This trait is that source, and it lives here rather than in
/// `htui-core` so invariant 10 holds in both directions: the engine never names a concrete store,
/// and `htui-store` never learns that an engine exists. `fake.rs` implements it for `MemStore`
/// (T3); `htui` implements it for `Backend` at milestone 6, where the wiring already belongs.
///
/// `app_setting` is deliberately **not** a fifth method (blueprint A-2): those reads are inherent
/// too, so [`resolve`] takes the resolved map as a parameter, the shape
/// `htui_core::prompt::settings::resolve_budget` already uses.
///
/// Plain `async fn` with the targeted allow, mirroring `htui-core`'s own store seam
/// (`crates/htui-core/src/store/traits.rs:62`): the engine is already generic over `S: WriteStore`
/// and takes `G` beside it, so no `dyn GraphSource` is ever formed and a boxed-future alias in the
/// shape of `htui_agent::driver::DriverFuture` would buy an allocation per read and nothing else.
#[allow(async_fn_in_trait)]
pub trait GraphSource: Sync {
    /// The graph an item runs under — `item.step_graph_id`, else the kind's `default_graph_id`
    /// (`docs/ANA-2.md:274-276`) — with its phases in `position` order. `None` when the item, its
    /// kind or the graph is gone.
    ///
    /// # Errors
    /// The backend's own failures.
    async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>>;

    /// Rung 1 of ANA-2 §4.1's candidate chain (`:288`): the phase's `phase_agent` rows in
    /// `position` order.
    ///
    /// An implementation over `MemStore` answers empty unconditionally — the store holds no such
    /// table — which is why T3's fake folds its stand-in candidates in here (plan D20).
    ///
    /// # Errors
    /// The backend's own failures.
    async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>>;

    /// One `prompt_template` by `(project, name)`: the pinned `version` when there is one, else
    /// the highest. `None` when the pin cannot be honoured.
    ///
    /// # Errors
    /// The backend's own failures.
    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> Result<Option<PromptTemplate>>;

    /// One `agent` row, for the denormalised `agent_name` of ANA-2 §5.1 and for
    /// `htui_agent::registry::caps_for`, which the stage-1 interlock calls before a driver exists
    /// to ask (plan D6).
    ///
    /// # Errors
    /// The backend's own failures.
    async fn agent(&self, id: AgentId) -> Result<Option<Agent>>;
}

/// What [`resolve`] answers: the snapshot and the scope, ready for
/// `NewRun` (`crates/htui-core/src/model/run.rs:352-378`).
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// `run.graph_snapshot`, with [`topology`] already computed.
    pub snapshot: GraphSnapshot,
    /// `run.repo_scope` (ANA-2 §4.7), from [`resolve_scope`].
    pub repo_scope: Vec<RepoId>,
}

/// Why a graph could not be resolved into a snapshot.
///
/// Every variant is a refusal before a run row exists, which is the cheap end of ANA-2 §4.2's
/// "fail before a token is spent".
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// Neither the item nor its kind names a graph (`docs/ANA-2.md:274-276`).
    #[error("item {0} has no step graph")]
    NoGraph(ItemId),
    /// A phase names a template the project does not hold at any version.
    #[error("phase `{phase}` names template `{name}` which project {project} does not hold")]
    NoTemplate {
        /// `step_graph_phase.name`.
        phase: String,
        /// `step_graph_phase.template_name`.
        name: String,
        /// The project searched.
        project: ProjectId,
    },
    /// ANA-2 §4.1 rung 4: `phase_agent` is empty, the project names no `default_agent_id`, and
    /// nothing else may be guessed (`docs/ANA-2.md:288`).
    #[error("phase `{phase}` has no candidate agent (ANA-2 §4.1 rung 4)")]
    NoCandidate {
        /// `step_graph_phase.name`.
        phase: String,
    },
    /// A candidate names an `agent` row that is not there, so `agent_name` cannot be denormalised.
    #[error("phase `{phase}` candidate {agent} names no agent row")]
    NoAgentRow {
        /// `step_graph_phase.name`.
        phase: String,
        /// The `agent.id` that resolved to nothing.
        agent: AgentId,
    },
    /// `local` isolation shares the maintainer's own working tree, so two candidates would write
    /// over each other (ANA-2 §4.6).
    #[error("phase `{phase}` is `local` with fan_out {fan_out} (ANA-2 §4.6)")]
    LocalFanOut {
        /// `step_graph_phase.name`.
        phase: String,
        /// `step_graph_phase.fan_out`.
        fan_out: i32,
    },
    /// Plan D14: an empty `repo_scope` overlaps nothing, because `'{}' && x` is false in Postgres,
    /// while ANA-2 §4.7 intends an empty `touched_paths` to overlap the whole primary repo. An
    /// item queued with an empty scope would silently defeat `claim_run`'s admission predicate, so
    /// resolution — which is where paths become a scope — is where the hole is closed.
    #[error("item {item} names primary repo {repo}; an empty repo_scope is refused (plan D14)")]
    EmptyScopeWithPrimary {
        /// The item being queued.
        item: ItemId,
        /// The project's `is_primary` repo.
        repo: RepoId,
    },
    /// The store said no.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// `"sha256:"` + hex over `serde_json::to_string` of the **typed** phase slice (plan D9).
///
/// ANA-2 defines the hash as "over the canonical serialisation of `phases[]`" (`:1479`) and never
/// defines "canonical", so this is where it is defined: the envelope (`v`, `graph`, `mode`,
/// `settings`) is excluded, field order is [`SnapshotPhase`]'s declaration order, and nulls are
/// emitted because no field carries `skip_serializing_if`. Adding a `SnapshotPhase` field changes
/// every digest and is therefore a `GraphSnapshot::V` question.
///
/// **Never route this through `serde_json::Value`.** `serde_json`'s `preserve_order` feature is
/// enabled in a whole-workspace build — `schemars` ← `agent-client-protocol-schema` ←
/// `htui-agent`, and Cargo feature unification hands it to `htui-core` too — and disabled under
/// `cargo test -p htui-orch`. A `Value` round trip therefore serialises in declaration order in
/// one build and alphabetical order in the other, yielding two digests for one graph. Serialising
/// the struct directly is immune, and this is a measured trap rather than a style preference.
#[must_use]
pub fn topology(phases: &[SnapshotPhase]) -> String {
    let json = serde_json::to_string(phases)
        .expect("SnapshotPhase serialises: no map keys, no non-finite numbers");
    format!("sha256:{}", htui_core::prompt::digest::sha256_hex(&json))
}

/// `run.repo_scope` for an item (ANA-2 §4.7, plan D14).
///
/// `requested` `Some(scope)` is honoured as given — the caller has already resolved
/// `item.touched_paths` to repos — except that an empty one is refused when the project has a
/// primary repo. `None` resolves to the primary repo alone, or to nothing when the project has no
/// repository at all, which is the only case an empty scope is legitimate in.
///
/// # Errors
/// [`ResolveError::EmptyScopeWithPrimary`] for an explicitly empty scope on an item whose project
/// has an `is_primary` repo.
pub fn resolve_scope(
    item: &Item,
    repos: &[Repo],
    requested: Option<&[RepoId]>,
) -> std::result::Result<Vec<RepoId>, ResolveError> {
    let primary = repos.iter().find(|repo| repo.is_primary);
    match (requested, primary) {
        (Some([]), Some(repo)) => Err(ResolveError::EmptyScopeWithPrimary {
            item: item.id,
            repo: repo.id,
        }),
        (Some(scope), _) => Ok(scope.to_vec()),
        (None, Some(repo)) => Ok(vec![repo.id]),
        (None, None) => Ok(Vec::new()),
    }
}

/// Builds ANA-2 §5.1's snapshot from the live graph, walking §4.1's chains once per field.
///
/// `app` is the resolved `app_setting` map — `MemStore::app_settings()` in a harness,
/// `Backend::app_settings()` at milestone 6 — passed rather than read through [`GraphSource`],
/// because the `app_setting` reads are inherent too (blueprint A-2).
///
/// `mode` is recorded on the snapshot so it is self-contained. It does **not** move
/// `gate_effective`: §4.10's auto-mode gate downgrade (`R-ORCH-6`) is a later milestone's, and
/// until it exists `gate_effective` equals `gate` for every phase in either mode.
///
/// # Errors
/// Any [`ResolveError`]; a missing `project` row is [`StoreError::NotFound`].
pub async fn resolve<S: ReadStore + WriteStore, G: GraphSource>(
    store: &S,
    source: &G,
    item: &Item,
    mode: RunMode,
    app: &BTreeMap<String, Value>,
    requested_scope: Option<&[RepoId]>,
) -> std::result::Result<Resolved, ResolveError> {
    let resolved = source
        .resolve_graph(item.id)
        .await?
        .ok_or(ResolveError::NoGraph(item.id))?;
    let project = store
        .project(item.project_id)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "project",
            id: item.project_id.to_string(),
        })?;
    let settings = project_settings(&project);

    // `UNIQUE (graph_id, position)` permits gaps, so the snapshot builder is what makes positions
    // dense `0..n-1` (`docs/ANA-2.md:269-272`). Sorting first means a gapped or out-of-order table
    // still yields the graph's own order rather than the store's.
    let mut rows = resolved.phases;
    rows.sort_by_key(|row| row.phase.position);

    let mut phases = Vec::with_capacity(rows.len());
    for (dense, row) in rows.iter().enumerate() {
        let position = i32::try_from(dense).map_err(|_| {
            StoreError::Constraint(format!(
                "graph {} has more phases than i32",
                resolved.graph.id
            ))
        })?;
        phases.push(
            snapshot_phase(
                source,
                &row.phase,
                &row.agents,
                position,
                project.id,
                &settings,
                app,
            )
            .await?,
        );
    }

    let snapshot = GraphSnapshot {
        v: GraphSnapshot::V,
        graph: SnapshotGraph {
            id: resolved.graph.id,
            name: resolved.graph.name,
            is_override: resolved.graph.is_override,
        },
        topology: topology(&phases),
        mode,
        phases,
        settings: SnapshotSettings {
            default_isolation: settings.default_isolation,
            per_token_cap_run: settings.per_token_cap_run,
            per_token_cap_batch: settings.per_token_cap_batch,
            max_fan_out: app_u32(app, "max_fan_out").unwrap_or(DEFAULT_MAX_FAN_OUT),
            max_agents_per_run: app_u32(app, "max_agents_per_run")
                .unwrap_or(DEFAULT_MAX_AGENTS_PER_RUN),
        },
    };

    let repos = store.repos(item.project_id).await?;
    let repo_scope = resolve_scope(item, &repos, requested_scope)?;
    Ok(Resolved {
        snapshot,
        repo_scope,
    })
}

/// `<item.key>-override`: a clone deep over `step_graph_phase` and **never** over `skill_binding`.
///
/// A phase binding is keyed `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`, so
/// copying bindings onto cloned phase ids would silently double every project binding the item's
/// phases inherit (`docs/ANA-2.md:290-295`, PRD `:399`). An override therefore starts with
/// project-level bindings only, and a phase binding on an override graph is created explicitly.
///
/// **Two halves of ANA-2's clone are owed to writers that do not exist yet** (blueprint R-6):
/// `WriteStore` has no `phase_agent` writer at all, and `step_graph.is_override` is settable by no
/// writer either — `NewStepGraph` has no field for it and `StepGraphPatch` says so in its own doc
/// comment. The clone this function performs is complete with respect to the seam it has.
/// Likewise **re-override** (ANA-2 `:297-300`: delete the existing override's phases and re-clone,
/// leaving `item.step_graph_id` alone) needs a phase deleter `WriteStore` does not carry; until it
/// does, a second call earns `create_step_graph`'s own `(project_id, name)` `Constraint`, which is
/// a refusal rather than a wrong answer.
///
/// # Errors
/// [`ResolveError::NoGraph`] when the item resolves to no graph; [`ResolveError::Store`] for the
/// store's own refusals, including a `Constraint` when the item's `version` has moved under the
/// caller.
pub async fn override_graph<S: WriteStore, G: GraphSource>(
    store: &S,
    source: &G,
    item: &Item,
) -> std::result::Result<StepGraph, ResolveError> {
    let resolved = source
        .resolve_graph(item.id)
        .await?
        .ok_or(ResolveError::NoGraph(item.id))?;

    let clone = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            project_id: item.project_id,
            name: format!("{}-override", item.key),
            description: format!(
                "Per-item override of `{}` for {}",
                resolved.graph.name, item.key
            ),
        })
        .await?;

    for row in &resolved.phases {
        store
            .create_phase(&StepGraphPhase {
                id: PhaseId::new(),
                graph_id: clone.id,
                ..row.phase.clone()
            })
            .await?;
    }

    // The item is repointed last, so a failure part-way through leaves an orphan graph rather than
    // an item naming a half-cloned one.
    let outcome = store
        .update_item(
            item.id,
            item.version,
            ItemPatch {
                step_graph_id: Some(Some(clone.id)),
                author_id: item.created_by,
                reason: "graph_override".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await?;
    match outcome {
        UpdateOutcome::Updated(_) => Ok(clone),
        UpdateOutcome::Diverged { head, .. } => {
            Err(ResolveError::Store(StoreError::Constraint(format!(
                "item {} moved to version {} while an override graph was being cloned",
                item.key, head.version
            ))))
        }
    }
}

/// `project.settings` as ANA-2 §4.7 reads it; a blob that does not decode is read as defaults
/// rather than as a failure, which is [`ProjectSettings`]'s own rule (every field defaults, so
/// `'{}'` decodes).
fn project_settings(project: &Project) -> ProjectSettings {
    serde_json::from_value(project.settings.clone()).unwrap_or_default()
}

/// One `app_setting` value as a positive `i64`.
///
/// "Positive or it did not answer" is `htui_core::prompt::settings`'s rule for the same table
/// (`crates/htui-core/src/prompt/settings.rs:617-623`): absent, `null`, a string, a bool, zero and
/// negative all mean the rung is silent, so a planted `0` falls through to the next rung rather
/// than pinning a budget of nothing.
fn app_positive(app: &BTreeMap<String, Value>, key: &str) -> Option<i64> {
    app.get(key).and_then(Value::as_i64).filter(|n| *n > 0)
}

/// [`app_positive`] narrowed to `u32`; a value too large for the column is read as silence.
fn app_u32(app: &BTreeMap<String, Value>, key: &str) -> Option<u32> {
    app_positive(app, key).and_then(|n| u32::try_from(n).ok())
}

/// [`app_positive`] narrowed to `i32`.
fn app_i32(app: &BTreeMap<String, Value>, key: &str) -> Option<i32> {
    app_positive(app, key).and_then(|n| i32::try_from(n).ok())
}

/// One phase of the live graph, with every §4.1 chain walked (`docs/ANA-2.md:281-288`).
async fn snapshot_phase<G: GraphSource>(
    source: &G,
    phase: &StepGraphPhase,
    rung_one: &[PhaseAgent],
    position: i32,
    project: ProjectId,
    settings: &ProjectSettings,
    app: &BTreeMap<String, Value>,
) -> std::result::Result<SnapshotPhase, ResolveError> {
    let isolation = phase.isolation.unwrap_or(settings.default_isolation);
    if isolation == Isolation::Local && phase.fan_out > 1 {
        return Err(ResolveError::LocalFanOut {
            phase: phase.name.clone(),
            fan_out: phase.fan_out,
        });
    }

    let version = match phase.template_version {
        Some(pinned) => pinned,
        None => {
            source
                .prompt_template(project, &phase.template_name, None)
                .await?
                .ok_or_else(|| ResolveError::NoTemplate {
                    phase: phase.name.clone(),
                    name: phase.template_name.clone(),
                    project,
                })?
                .version
        }
    };

    Ok(SnapshotPhase {
        position,
        name: phase.name.clone(),
        fan_out: phase.fan_out,
        gate: phase.gate,
        // `R-ORCH-6`'s auto-mode downgrade is §4.10's and is not implemented here, so nothing has
        // downgraded this gate and the two agree by construction.
        gate_effective: phase.gate,
        gate_hard: phase.gate_hard,
        retry_limit: phase.retry_limit,
        input_kinds: phase.input_kinds.clone(),
        output_kind: phase.output_kind.clone(),
        isolation,
        command_queue: phase.command_queue,
        verify_command: phase.verify_command.clone(),
        // `StepGraphPhase` carries no `deadline_seconds` field at HEAD though `0003` added the
        // column, so the chain starts one rung down, at the project (blueprint H-14). Adding the
        // field is a model change and would touch `.sqlx`; it is recorded rather than fixed.
        deadline_seconds: Some(
            settings
                .step_deadline_seconds
                .or_else(|| app_u32(app, "step_deadline_seconds"))
                .unwrap_or(DEFAULT_DEADLINE_SECONDS),
        ),
        template: SnapshotTemplate {
            name: phase.template_name.clone(),
            version,
        },
        token_budget: phase
            .token_budget
            .filter(|n| *n > 0)
            .or_else(|| settings.token_budget.filter(|n| *n > 0))
            .or_else(|| app_i32(app, "token_budget")),
        candidates: candidates(source, phase, rung_one, settings).await?,
        // `StepGraphPhase` carries no `judge_agent_id` / `judge_model` either (blueprint H-14), so
        // this chain also starts at the project rung; `None` means human selection (§4.5).
        judge: match settings.judge_agent_id {
            None => None,
            Some(id) => Some(SnapshotJudge {
                agent_id: id,
                agent_name: agent_name(source, &phase.name, id).await?,
                model: None,
            }),
        },
    })
}

/// ANA-2 §4.1's candidate chain (`docs/ANA-2.md:288`), the rungs this crate owns.
///
/// Rung 1 is `phase_agent` in `position` order; rung 2 is `project.settings.default_agent_id`;
/// rung 4 is the refusal. **Rung 3 — "the single enabled agent on the box" — is deliberately not
/// here**: it needs a box-scoped agent listing, which [`GraphSource`] does not carry and which
/// would be a fifth method for one fallback. The implementation of [`GraphSource::phase_agents`]
/// folds it in instead, which is also where T3's fake puts its stand-in candidates (plan D20,
/// blueprint A-1). A phase that reaches rung 4 is a named refusal and never a silent empty walk.
async fn candidates<G: GraphSource>(
    source: &G,
    phase: &StepGraphPhase,
    rung_one: &[PhaseAgent],
    settings: &ProjectSettings,
) -> std::result::Result<Vec<SnapshotCandidate>, ResolveError> {
    // `ResolvedGraph` already carries the phase's `phase_agent` rows on backends that hold the
    // table; asking the source again is how a backend that answers them separately (or a fake that
    // stands them in) gets its say.
    let mut rows = source.phase_agents(phase.id).await?;
    if rows.is_empty() {
        rows = rung_one.to_vec();
    }
    rows.sort_by_key(|row| row.position);

    if !rows.is_empty() {
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(SnapshotCandidate {
                agent_id: row.agent_id,
                agent_name: agent_name(source, &phase.name, row.agent_id).await?,
                model: row.model,
            });
        }
        return Ok(out);
    }

    let Some(id) = settings.default_agent_id else {
        return Err(ResolveError::NoCandidate {
            phase: phase.name.clone(),
        });
    };
    let agent = source
        .agent(id)
        .await?
        .ok_or_else(|| ResolveError::NoAgentRow {
            phase: phase.name.clone(),
            agent: id,
        })?;
    // `SnapshotCandidate.model` is not nullable: a candidate the engine cannot name a model for is
    // not a candidate, so the project default falls through to rung 4 rather than to an empty
    // string the driver would then be handed.
    let model = agent
        .default_model
        .clone()
        .or_else(|| agent.models.first().cloned())
        .ok_or_else(|| ResolveError::NoCandidate {
            phase: phase.name.clone(),
        })?;
    Ok(vec![SnapshotCandidate {
        agent_id: agent.id,
        agent_name: agent.name,
        model,
    }])
}

/// `agent.name`, denormalised onto the snapshot so an offline reader needs no registry (§5.1).
async fn agent_name<G: GraphSource>(
    source: &G,
    phase: &str,
    id: AgentId,
) -> std::result::Result<String, ResolveError> {
    source
        .agent(id)
        .await?
        .map(|agent| agent.name)
        .ok_or_else(|| ResolveError::NoAgentRow {
            phase: phase.to_owned(),
            agent: id,
        })
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{DemoData, demo_data, ids};
    use htui_core::model::{Gate, NewRepo, RepoId};
    use htui_core::store::MemStore;
    use serde_json::json;

    use super::{
        Agent, AgentId, BTreeMap, GraphSource, Isolation, Item, ItemId, PhaseAgent, PhaseId,
        ProjectId, PromptTemplate, ReadStore, ResolveError, Resolved, ResolvedGraph, Result,
        RunMode, StepGraphPhase, Value, WriteStore, override_graph, resolve,
    };

    /// The digest of the seeded `feature` graph, resolved against the demo fixture with one
    /// `claude`/`sonnet` candidate per phase. Computed once and pasted, which is the only way a
    /// pinned vector means anything.
    const FEATURE_TOPOLOGY: &str =
        "sha256:ce0b489ff772a6a1c84c7590dcf6ba60b68121716f6897decf628115bd7c959b";

    /// What a change to [`FEATURE_TOPOLOGY`] means, said where it will be read.
    const TOPOLOGY_MOVED: &str = "\
the `topology` digest of the seeded `feature` graph moved. It is sha256 over \
`serde_json::to_string` of the **typed** `&[SnapshotPhase]` in declaration order (plan D9), so it \
moves only when the phase struct gains or loses a field, when the seeded graph changes, or when a \
resolved value does — the template versions, the `7200` deadline default, the denormalised \
`agent_name`, the project's `token_budget`. Every one of those also changes the digest a *stored* \
snapshot would compute for the same graph, so re-pinning this literal is a `GraphSnapshot::V` \
question and not a test fix. Decide the version bump first, then paste the new digest.";

    /// A `GraphSource` over `&MemStore` with one scripted candidate per phase.
    ///
    /// T3's `FakeGraphSource` is the real one; this is the smallest thing that makes `graph.rs`
    /// testable on its own, and it exists for the reason plan D20 gives: `MemStore::phase_agents`
    /// answers empty unconditionally (`crates/htui-core/src/store/mem.rs:469-472`) and every demo
    /// agent is `enabled`, so rungs 1 and 3 of §4.1's chain are both empty here and a resolution
    /// with no stand-in would refuse every phase for a reason that is about the fixture rather
    /// than about the walk.
    struct TestSource<'a> {
        store: &'a MemStore,
        candidates: Vec<(AgentId, &'static str)>,
    }

    impl<'a> TestSource<'a> {
        /// One `claude`/`sonnet` candidate per phase — the pair `RUN_1`'s own steps carry.
        fn claude(store: &'a MemStore) -> Self {
            Self {
                store,
                candidates: vec![(ids::AGENT_CLAUDE, "sonnet")],
            }
        }

        /// No rung-1 candidate at all, so resolution falls through to the project rung.
        fn barren(store: &'a MemStore) -> Self {
            Self {
                store,
                candidates: Vec::new(),
            }
        }
    }

    impl GraphSource for TestSource<'_> {
        async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
            self.store.resolve_graph(item).await
        }

        async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
            Ok(self
                .candidates
                .iter()
                .enumerate()
                .map(|(position, (agent_id, model))| PhaseAgent {
                    phase_id: phase,
                    position: i32::try_from(position).expect("a handful of candidates"),
                    agent_id: *agent_id,
                    model: (*model).to_owned(),
                })
                .collect())
        }

        async fn prompt_template(
            &self,
            project: ProjectId,
            name: &str,
            version: Option<i32>,
        ) -> Result<Option<PromptTemplate>> {
            self.store.prompt_template(project, name, version).await
        }

        async fn agent(&self, id: AgentId) -> Result<Option<Agent>> {
            Ok(self
                .store
                .agents()
                .await?
                .into_iter()
                .map(|row| row.agent)
                .find(|agent| agent.id == id))
        }
    }

    /// The demo fixture with `htui`'s `project.settings` replaced.
    fn store_with_settings(settings: Value) -> MemStore {
        store_with(|data| {
            for project in &mut data.projects {
                if project.id == ids::PROJECT_HTUI {
                    project.settings = settings.clone();
                }
            }
        })
    }

    /// The demo fixture, edited.
    fn store_with(edit: impl FnOnce(&mut DemoData)) -> MemStore {
        let mut data = demo_data();
        edit(&mut data);
        MemStore::from_demo(data)
    }

    async fn feat_1(store: &MemStore) -> Item {
        store
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture holds HTUI_FEAT-1")
    }

    /// `resolve` with an empty `app_setting` map and no requested scope.
    async fn resolve_feat(
        store: &MemStore,
        source: &TestSource<'_>,
    ) -> std::result::Result<Resolved, ResolveError> {
        let item = feat_1(store).await;
        resolve(
            store,
            source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
        )
        .await
    }

    /// The test this module was written against (plan D9, blueprint H-2/H-17).
    ///
    /// The vector is asserted in a crate-scoped run *and* in a workspace run, which is what makes
    /// the `preserve_order` divergence visible if it ever returns: `serde_json`'s `preserve_order`
    /// is feature-unified **on** under `cargo test --workspace` and **off** under
    /// `cargo test -p htui-orch`, so a digest taken through `serde_json::Value` would differ
    /// between the two invocations while both stayed green on their own.
    #[tokio::test]
    async fn feature_snapshot_topology_is_pinned() {
        let store = MemStore::demo();
        let resolved = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves");
        assert_eq!(
            resolved.snapshot.topology, FEATURE_TOPOLOGY,
            "{TOPOLOGY_MOVED}"
        );
        assert_eq!(
            resolved.snapshot.topology,
            super::topology(&resolved.snapshot.phases),
            "the snapshot carries the digest of its own phases"
        );
        assert_eq!(resolved.snapshot.v, 1, "{TOPOLOGY_MOVED}");
    }

    /// `UNIQUE (graph_id, position)` permits gaps; the snapshot builder is what closes them
    /// (`docs/ANA-2.md:269-272`).
    #[tokio::test]
    async fn positions_are_dense() {
        let store = store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id == ids::GRAPH_HTUI_FEAT {
                    // 0,1,2,3 becomes 0,10,20,30: same order, four gaps.
                    phase.position *= 10;
                }
            }
        });
        let resolved = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("a gapped graph still resolves");
        let positions: Vec<i32> = resolved
            .snapshot
            .phases
            .iter()
            .map(|phase| phase.position)
            .collect();
        let names: Vec<&str> = resolved
            .snapshot
            .phases
            .iter()
            .map(|phase| phase.name.as_str())
            .collect();
        assert_eq!(positions, [0, 1, 2, 3]);
        assert_eq!(names, ["prd", "plan", "implement", "review"]);
    }

    /// One assertion per row of ANA-2 §4.1's chain table (`docs/ANA-2.md:281-288`), against the
    /// fixture's own values.
    #[tokio::test]
    async fn field_chains_walk_phase_project_app() {
        let store = MemStore::demo();
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        let prd = &snapshot.phases[0];
        let implement = &snapshot.phases[2];

        // Copied straight off the row.
        assert_eq!(prd.fan_out, 1);
        assert_eq!(prd.retry_limit, 1);
        assert_eq!(prd.output_kind, "prd");
        assert_eq!(prd.verify_command, None, "every seeded phase has none");
        assert_eq!(implement.input_kinds, ["plan", "review"]);
        assert!(
            prd.gate_hard,
            "`feature.prd` is one of the three seeded true"
        );
        assert!(!implement.gate_hard);

        // `gate_effective`: nothing downgrades a gate this milestone.
        assert_eq!(prd.gate, Gate::Always);
        assert_eq!(prd.gate_effective, prd.gate);

        // `isolation`: the phase says nothing, so the project rung answers, and the seed leaves
        // that at the built-in `worktree`.
        assert_eq!(prd.isolation, Isolation::Worktree);
        assert_eq!(snapshot.settings.default_isolation, Isolation::Worktree);

        // `template_version`: the phase pins none, so the latest in the project wins — which the
        // seed writes as version 1.
        assert_eq!(prd.template.name, "prd");
        assert_eq!(prd.template.version, 1);

        // `token_budget`: the phase carries none and the demo project's settings blob carries
        // 120_000, so the project rung answers.
        assert_eq!(prd.token_budget, Some(120_000));

        // `deadline_seconds`: no phase column exists at HEAD (blueprint H-14) and neither the
        // project nor `app_setting` answers, so the built-in two hours does.
        assert_eq!(prd.deadline_seconds, Some(7200));

        // `candidates`: rung 1, with `agent_name` denormalised off the registry.
        assert_eq!(prd.candidates.len(), 1);
        assert_eq!(prd.candidates[0].agent_id, ids::AGENT_CLAUDE);
        assert_eq!(prd.candidates[0].agent_name, "claude");
        assert_eq!(prd.candidates[0].model, "sonnet");

        // `judge_agent_id`: no phase column and no project setting, so none — human selection.
        assert_eq!(prd.judge, None);

        // The `app_setting` rungs of the envelope.
        assert_eq!(snapshot.settings.max_fan_out, 4);
        assert_eq!(snapshot.settings.max_agents_per_run, 6);
        assert_eq!(snapshot.mode, RunMode::Manual);
        assert_eq!(snapshot.graph.id, ids::GRAPH_HTUI_FEAT);
        assert!(!snapshot.graph.is_override);
    }

    /// The project rung of four chains at once, driven through `project.settings` — which no
    /// writer on the seam can reach, because `set_setting`'s `SettingKey` is a closed enum and
    /// `update_project` never touches the column (blueprint F-F).
    #[tokio::test]
    async fn the_project_rung_answers_when_the_phase_does_not() {
        let store = store_with_settings(json!({
            "default_isolation": "copy",
            "token_budget": 42,
            "step_deadline_seconds": 600,
            "judge_agent_id": ids::AGENT_AGY,
            "per_token_cap_run": 7,
            "per_token_cap_batch": 9,
        }));
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        let prd = &snapshot.phases[0];

        assert_eq!(prd.isolation, Isolation::Copy);
        assert_eq!(prd.token_budget, Some(42));
        assert_eq!(prd.deadline_seconds, Some(600));
        let judge = prd.judge.as_ref().expect("the project names a judge");
        assert_eq!(judge.agent_id, ids::AGENT_AGY);
        assert_eq!(judge.agent_name, "agy");
        assert_eq!(judge.model, None, "no phase column carries a judge model");
        assert_eq!(snapshot.settings.default_isolation, Isolation::Copy);
        assert_eq!(snapshot.settings.per_token_cap_run, Some(7));
        assert_eq!(snapshot.settings.per_token_cap_batch, Some(9));
        assert_ne!(
            snapshot.topology, FEATURE_TOPOLOGY,
            "a resolved value moving is a different topology, which is the point of the digest"
        );
    }

    /// The `app_setting` rung, and the "positive or the rung is silent" rule the prompt settings
    /// resolver already applies to the same table.
    #[tokio::test]
    async fn the_app_rung_answers_when_the_project_does_not() {
        let store = store_with_settings(json!({}));
        let item = feat_1(&store).await;
        let app: BTreeMap<String, Value> = [
            ("token_budget".to_owned(), json!(9_000)),
            ("step_deadline_seconds".to_owned(), json!(300)),
            ("max_fan_out".to_owned(), json!(2)),
            ("max_agents_per_run".to_owned(), json!(3)),
        ]
        .into_iter()
        .collect();
        let snapshot = resolve(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Auto,
            &app,
            None,
        )
        .await
        .expect("the seeded feature graph resolves")
        .snapshot;

        assert_eq!(snapshot.phases[0].token_budget, Some(9_000));
        assert_eq!(snapshot.phases[0].deadline_seconds, Some(300));
        assert_eq!(snapshot.settings.max_fan_out, 2);
        assert_eq!(snapshot.settings.max_agents_per_run, 3);
        assert_eq!(snapshot.mode, RunMode::Auto);

        let silent: BTreeMap<String, Value> = [
            ("token_budget".to_owned(), json!(0)),
            ("step_deadline_seconds".to_owned(), json!("600")),
            ("max_fan_out".to_owned(), json!(-1)),
        ]
        .into_iter()
        .collect();
        let snapshot = resolve(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Manual,
            &silent,
            None,
        )
        .await
        .expect("the seeded feature graph resolves")
        .snapshot;
        assert_eq!(snapshot.phases[0].token_budget, None);
        assert_eq!(snapshot.phases[0].deadline_seconds, Some(7200));
        assert_eq!(snapshot.settings.max_fan_out, 4);
    }

    /// Rung 2 of the candidate chain: `project.settings.default_agent_id` with the agent's own
    /// `default_model`.
    #[tokio::test]
    async fn the_project_default_agent_is_the_second_candidate_rung() {
        let store = store_with_settings(json!({ "default_agent_id": ids::AGENT_AGY }));
        let snapshot = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect("the project default answers")
            .snapshot;
        let candidates = &snapshot.phases[0].candidates;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].agent_id, ids::AGENT_AGY);
        assert_eq!(candidates[0].agent_name, "agy");
        assert_eq!(candidates[0].model, "gemini-3.7-flash-high");
    }

    /// Rung 4: refuse, by name. A phase that resolved to no candidate and said nothing would make
    /// the engine refuse every run for a reason the operator could not read.
    #[tokio::test]
    async fn no_candidate_is_a_named_refusal() {
        let store = MemStore::demo();
        let error = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect_err("the demo fixture answers no rung");
        assert_eq!(
            error,
            ResolveError::NoCandidate {
                phase: "prd".to_owned()
            }
        );

        // `claude`'s seed row carries neither a `default_model` nor any `models`, so the project
        // rung cannot name a model and falls through to the same refusal rather than handing the
        // driver an empty string.
        let store = store_with_settings(json!({ "default_agent_id": ids::AGENT_CLAUDE }));
        let error = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect_err("a candidate with no model is not a candidate");
        assert_eq!(
            error,
            ResolveError::NoCandidate {
                phase: "prd".to_owned()
            }
        );
    }

    /// A phase naming a template the project does not hold cannot be resolved to a version, and a
    /// snapshot with an unresolvable template is a run that fails at stage 3.
    #[tokio::test]
    async fn a_missing_template_is_refused() {
        let store = store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id == ids::GRAPH_HTUI_FEAT && phase.name == "plan" {
                    phase.template_name = "nothing-seeds-this".to_owned();
                }
            }
        });
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("the template is not in the project");
        assert_eq!(
            error,
            ResolveError::NoTemplate {
                phase: "plan".to_owned(),
                name: "nothing-seeds-this".to_owned(),
                project: ids::PROJECT_HTUI,
            }
        );
    }

    /// `local` isolation is the maintainer's own working tree, so two candidates in it would write
    /// over each other (ANA-2 §4.6).
    #[tokio::test]
    async fn local_with_fan_out_is_refused() {
        let store = store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id == ids::GRAPH_HTUI_FEAT && phase.name == "implement" {
                    phase.isolation = Some(Isolation::Local);
                    phase.fan_out = 3;
                }
            }
        });
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("`local` cannot fan out");
        assert_eq!(
            error,
            ResolveError::LocalFanOut {
                phase: "implement".to_owned(),
                fan_out: 3,
            }
        );

        // `local` at `fan_out = 1` is the ordinary case and resolves.
        let store = store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id == ids::GRAPH_HTUI_FEAT && phase.name == "implement" {
                    phase.isolation = Some(Isolation::Local);
                }
            }
        });
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("one local step is fine")
            .snapshot;
        assert_eq!(snapshot.phases[2].isolation, Isolation::Local);
    }

    /// Plan D14, and the only case an empty scope is legitimate in.
    #[tokio::test]
    async fn empty_scope_with_primary_is_refused() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let source = TestSource::claude(&store);

        // The demo fixture seeds no `repo` rows at all, so there is no primary to overlap with and
        // an empty scope is what the item honestly has.
        let resolved = resolve(
            &store,
            &source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            Some(&[]),
        )
        .await
        .expect("a project with no repository resolves to an empty scope");
        assert_eq!(resolved.repo_scope, Vec::<RepoId>::new());

        let repo = store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the fixture project takes a repo");

        let error = resolve(
            &store,
            &source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            Some(&[]),
        )
        .await
        .expect_err("an empty scope overlaps nothing, which defeats `claim_run`");
        assert_eq!(
            error,
            ResolveError::EmptyScopeWithPrimary {
                item: item.id,
                repo: repo.id,
            }
        );

        // No requested scope resolves to the primary repo, which is §4.7's "an empty
        // `touched_paths` overlaps the whole primary repo" expressed as a scope.
        let resolved = resolve(
            &store,
            &source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
        )
        .await
        .expect("the primary answers");
        assert_eq!(resolved.repo_scope, vec![repo.id]);
    }

    /// ANA-2 §4.1: the clone is deep over `step_graph_phase` and **never** over `skill_binding`,
    /// because a phase binding keyed `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`
    /// would double every project binding the item's phases inherit.
    #[tokio::test]
    async fn override_clone_leaves_bindings_alone() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let source = TestSource::claude(&store);

        let project_level = store
            .bound_skills(ids::PROJECT_HTUI, None)
            .await
            .expect("MemStore never fails a read");
        let bound_before = store
            .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
            .await
            .expect("MemStore never fails a read");
        assert_ne!(
            bound_before, project_level,
            "the fixture's one phase-level binding is what makes this test able to fail"
        );

        let clone = override_graph(&store, &source, &item)
            .await
            .expect("the item takes an override");
        assert_eq!(clone.name, format!("{}-override", item.key));

        let original: Vec<StepGraphPhase> = store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        let cloned: Vec<StepGraphPhase> = store
            .phases(clone.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(cloned.len(), original.len(), "every phase is copied");
        for (left, right) in original.iter().zip(&cloned) {
            assert_ne!(left.id, right.id, "a clone mints fresh phase ids");
            assert_eq!(left.name, right.name);
            assert_eq!(left.input_kinds, right.input_kinds);
            assert_eq!(left.gate, right.gate);
            assert_eq!(left.gate_hard, right.gate_hard);
            assert_eq!(left.template_name, right.template_name);
        }

        let implement = cloned
            .iter()
            .find(|phase| phase.name == "implement")
            .expect("the feature graph has an implement phase");
        assert_eq!(
            store
                .bound_skills(ids::PROJECT_HTUI, Some(implement.id))
                .await
                .expect("MemStore never fails a read"),
            project_level,
            "a cloned phase inherits the project's bindings and none of the original phase's"
        );
        assert_eq!(
            store
                .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
                .await
                .expect("MemStore never fails a read"),
            bound_before,
            "and the original phase's bindings are untouched"
        );

        let repointed = store
            .item(item.id)
            .await
            .expect("MemStore never fails a read")
            .expect("the item is still there");
        assert_eq!(repointed.step_graph_id, Some(clone.id));

        // Resolution now follows the override, and the two graphs have the same topology because
        // the clone copied every field the digest covers.
        let resolved = resolve(
            &store,
            &source,
            &repointed,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
        )
        .await
        .expect("the override resolves");
        assert_eq!(resolved.snapshot.graph.id, clone.id);
        assert_eq!(resolved.snapshot.topology, FEATURE_TOPOLOGY);
    }

    /// An item whose kind names no graph and which names none itself cannot be run.
    #[tokio::test]
    async fn an_item_with_no_graph_is_refused() {
        let store = store_with(|data| data.graphs.retain(|graph| graph.id != ids::GRAPH_HTUI_FEAT));
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("the kind's default graph is gone");
        assert_eq!(error, ResolveError::NoGraph(ids::HTUI_FEAT_1));
    }
}
