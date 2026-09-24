//! The orchestrator, driven from the store worker loop (MOD-4 milestone 6, plan D153).
//!
//! `htui-orch` never names `htui-store` (ANA-2 invariant 10), so everything that joins the two
//! lives here: the graph source over a [`Backend`], and — as the milestone lands — the runtime
//! that serves the orchestrator's requests on its own tasks beside `AgentRuntime`.

use std::collections::{BTreeMap, BTreeSet};

use htui_core::model::{
    Agent, AgentBox, AgentId, BoxId, DocumentHead, DocumentId, Item, ItemId, PhaseAgent, PhaseId,
    ProjectId, PromptTemplate, ResolvedGraph, Run, RunId, RunStep, StepId,
};
use htui_core::store::{ReadStore as _, Result as StoreResult, StoreError};
use htui_orch::command::{answer_gate_enabled, cancel_enabled, select_enabled};
use htui_orch::status::group_at;
use htui_orch::{
    Command, CommandOutcome, Cursor, EngineError, GateAnswer, GraphSource, Rest, accept_enabled,
    cleanup_enabled, close_out_enabled, cursor, phase_at, promote_enabled, retry_admitted,
    snapshot_of, start_enabled, unblock_enabled,
};
use htui_store::{Backend, DATABASE_UNREACHABLE};

/// Blueprint D209: the [`StoreRequest::name`](crate::store_worker::StoreRequest::name) of each
/// [`OrchRequest`], in [`OrchRequest`]'s order — the nine commands of [`Command`], then the
/// close-out preview and the cleanup retry. The status line reads `retry_step: …`, and the Runs
/// pane and the Chat tab match a `Failed` reply's `request` against this list.
pub const ORCH_NAMES: [&str; 11] = [
    "start_run",
    "answer_gate",
    "retry_step",
    "cancel_run",
    "select_fanout",
    "promote_step",
    "accept_artifact",
    "unblock",
    "close_out",
    "close_out_preview",
    "cleanup_run",
];

/// Plan D154: one orchestrator command or read.
#[derive(Debug, Clone)]
pub enum OrchRequest {
    /// One of ANA-2 §6.2's verbs, dispatched to the engine on a task of its own.
    Command(Command),
    /// Plan D167's first confirmation: what a close-out would write, read-only.
    CloseOutPreview {
        /// The item to close.
        item: ItemId,
    },
    /// Plan D177 (R-25): a manual retry of a terminal run's cleanup.
    Cleanup {
        /// The terminal run.
        run: RunId,
    },
}

impl OrchRequest {
    /// Blueprint D209: this request's entry of [`ORCH_NAMES`]. A `const fn`, because
    /// `StoreRequest::name` is one.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Command(command) => match command {
                Command::StartRun { .. } => ORCH_NAMES[0],
                Command::AnswerGate { .. } => ORCH_NAMES[1],
                Command::RetryStep { .. } => ORCH_NAMES[2],
                Command::CancelRun { .. } => ORCH_NAMES[3],
                Command::SelectFanout { .. } => ORCH_NAMES[4],
                Command::PromoteStep { .. } => ORCH_NAMES[5],
                Command::AcceptArtifact { .. } => ORCH_NAMES[6],
                Command::Unblock { .. } => ORCH_NAMES[7],
                Command::CloseOut { .. } => ORCH_NAMES[8],
            },
            Self::CloseOutPreview { .. } => ORCH_NAMES[9],
            Self::Cleanup { .. } => ORCH_NAMES[10],
        }
    }
}

/// The answer to one [`OrchRequest`], sent once at the request's own `seq` (R-41).
#[derive(Debug, Clone)]
pub enum OrchReply {
    /// A command's outcome.
    Done(Box<CommandOutcome>),
    /// To the Chat tab only (D165, D191): the promoted step and how its chat opens.
    Promoted {
        /// The promoted step, which keeps its id.
        step: StepId,
        /// Its run.
        run: RunId,
        /// `run_step.phase_name`.
        phase: String,
        /// `agent.name` of the step's agent.
        agent: String,
        /// `run_step.model`.
        model: Option<String>,
        /// Whether the chat resumes the step's own session or opens with the handoff prompt.
        via: Via,
    },
    /// [`OrchRequest::CloseOutPreview`]'s figures.
    CloseOutPreview(Box<htui_orch::closeout::Preview>),
    /// [`OrchRequest::Cleanup`] ran to its end.
    CleanedUp {
        /// The run cleaned up.
        run: RunId,
    },
}

/// How a promoted step's chat opens (blueprint D192).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// The step's own agent session, resumed.
    Resumed,
    /// A fresh session opened with the `handoff` prompt.
    Handoff,
}

/// Plan D172: one frame of an item's run stream. Frames are invalidations: the pane re-reads the
/// item's runs on each.
#[derive(Debug, Clone)]
pub struct RunFrame {
    /// The item whose runs changed.
    pub item: ItemId,
    /// The run that changed, when one did.
    pub run: Option<RunId>,
    /// What happened.
    pub kind: FrameKind,
}

impl RunFrame {
    /// The acknowledgement a `RunStream` subscription is answered with at once (blueprint §0a
    /// point 3, D183).
    #[must_use]
    pub const fn subscribed(item: ItemId) -> Self {
        Self {
            item,
            run: None,
            kind: FrameKind::Subscribed,
        }
    }
}

/// What a [`RunFrame`] reports.
#[derive(Debug, Clone)]
pub enum FrameKind {
    /// The subscription is live.
    Subscribed,
    /// A run was created, `queued`.
    Started,
    /// A session of `step` ended (the progress sink's `after_done`).
    SessionDone {
        /// The step whose session ended.
        step: StepId,
    },
    /// A walk rested.
    Rested(Rest),
    /// A sweep adopted the run; its walk resumes on a task of its own.
    Adopted,
    /// A command or walk failed, with the sentence.
    Error(String),
}

/// One action's enabling verdict: `Ok`, or the refusal's `Display` (D182, D184).
pub type Enabled = Result<(), String>;

/// Blueprint D182: every action's verdict for one item, from the engine's own guards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemActions {
    /// The item.
    pub item: ItemId,
    /// `item.key`, empty when the item could not be read.
    pub key: String,
    /// `R`: start a run.
    pub run: Enabled,
    /// `u`: unblock.
    pub unblock: Enabled,
    /// Close-out.
    pub close_out: Enabled,
    /// Per run.
    pub runs: BTreeMap<RunId, RunActions>,
    /// Per step.
    pub steps: BTreeMap<StepId, StepActions>,
}

/// The run-level verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunActions {
    /// Cancel the run.
    pub cancel: Enabled,
    /// Retry its terminal cleanup (R-25).
    pub cleanup: Enabled,
}

/// The step-level verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepActions {
    /// Approve the parked gate.
    pub approve: Enabled,
    /// Reject it with a note.
    pub reject: Enabled,
    /// Retry the step.
    pub retry: Enabled,
    /// Promote it to a chat.
    pub promote: Enabled,
    /// Accept a promoted step's artefact.
    pub accept: Enabled,
    /// Select it as a fan-out winner.
    pub select: Enabled,
    /// The newest head of the phase's `output_kind` the step produced.
    pub open: Result<DocumentId, String>,
}

/// Blueprint D206: the steps a chat of this process is live on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveChats(BTreeSet<StepId>);

impl LiveChats {
    /// The set of these steps.
    pub fn of(steps: impl IntoIterator<Item = StepId>) -> Self {
        Self(steps.into_iter().collect())
    }

    /// Whether no chat is live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether a chat is live on `step`.
    #[must_use]
    pub fn contains(&self, step: StepId) -> bool {
        self.0.contains(&step)
    }
}

/// Blueprint D182, D184: every verdict for `item`, from the engine's own admission functions
/// over the rows the engine reads — the item, its runs with their steps, and its document heads.
///
/// Off the server (`writer()` is `None`) every command verdict is [`DATABASE_UNREACHABLE`]: the
/// rows still come from the mirror, but no command can run (D174). A snapshot that does not decode
/// refuses every step verdict of its run with that sentence.
///
/// # Errors
/// The store's own read failures, and [`StoreError::NotFound`] for an item a reachable server does
/// not hold.
pub async fn actions(
    backend: &Backend,
    item: ItemId,
    live: &LiveChats,
) -> StoreResult<ItemActions> {
    let online = backend.writer().is_some();
    let row = backend.item(item).await?;
    let row = match row {
        Some(row) => row,
        None if online => {
            return Err(StoreError::NotFound {
                entity: "item",
                id: item.to_string(),
            });
        }
        None => return Ok(unreachable_actions(item, String::new())),
    };
    let heads = backend.documents(item).await?;
    let mut runs = Vec::new();
    for summary in backend.runs(item).await? {
        let Some(run) = backend.run(summary.id).await? else {
            continue;
        };
        let steps = backend.run_steps(run.id).await?;
        runs.push((run, steps));
    }

    let mut actions = verdicts(&row, &runs, &heads, live);
    if !online {
        let offline = || Err(DATABASE_UNREACHABLE.to_owned());
        actions.run = offline();
        actions.unblock = offline();
        actions.close_out = offline();
        for verdict in actions.runs.values_mut() {
            *verdict = RunActions {
                cancel: offline(),
                cleanup: offline(),
            };
        }
        for verdict in actions.steps.values_mut() {
            verdict.approve = offline();
            verdict.reject = offline();
            verdict.retry = offline();
            verdict.promote = offline();
            verdict.accept = offline();
            verdict.select = offline();
        }
    }
    Ok(actions)
}

/// An item the mirror does not hold, off the server: nothing is enabled.
fn unreachable_actions(item: ItemId, key: String) -> ItemActions {
    let offline = || Err(DATABASE_UNREACHABLE.to_owned());
    ItemActions {
        item,
        key,
        run: offline(),
        unblock: offline(),
        close_out: offline(),
        runs: BTreeMap::new(),
        steps: BTreeMap::new(),
    }
}

/// The guards themselves, over rows already read.
fn verdicts(
    item: &Item,
    runs: &[(Run, Vec<RunStep>)],
    heads: &[DocumentHead],
    live: &LiveChats,
) -> ItemActions {
    let sentence = |err: EngineError| err.to_string();
    let mut actions = ItemActions {
        item: item.id,
        key: item.key.clone(),
        run: start_enabled(item).map_err(sentence),
        unblock: Ok(()),
        close_out: close_out_enabled(
            item,
            &runs.iter().map(|(run, _)| run.clone()).collect::<Vec<_>>(),
        )
        .map_err(sentence),
        runs: BTreeMap::new(),
        steps: BTreeMap::new(),
    };

    let mut active: Vec<(Run, Cursor)> = Vec::new();
    let mut unblock_refusal = None;
    for (run, steps) in runs {
        actions.runs.insert(
            run.id,
            RunActions {
                cancel: cancel_enabled(run).map_err(sentence),
                cleanup: cleanup_enabled(run).map_err(sentence),
            },
        );
        let snapshot = match snapshot_of(run) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                let refusal = err.to_string();
                if run.status.is_active() {
                    unblock_refusal.get_or_insert_with(|| refusal.clone());
                }
                for step in steps {
                    actions.steps.insert(step.id, refused_step(&refusal));
                }
                continue;
            }
        };
        if run.status.is_active() {
            active.push((run.clone(), cursor(&snapshot, steps)));
        }
        for step in steps {
            let phase = match phase_at(run.id, &snapshot, step.position) {
                Ok(phase) => phase,
                Err(err) => {
                    actions
                        .steps
                        .insert(step.id, refused_step(&err.to_string()));
                    continue;
                }
            };
            let open = newest_output(heads, &phase.output_kind, step.id);
            let has_output = open.is_ok();
            let select = if step.fanout_index >= 0 && phase.fan_out > 1 {
                select_enabled(
                    run,
                    &group_at(steps, step.position, step.attempt),
                    step.id,
                    step.position,
                    step.attempt,
                )
                .map_err(sentence)
            } else {
                Err(not_a_candidate(step.id))
            };
            actions.steps.insert(
                step.id,
                StepActions {
                    approve: answer_gate_enabled(step, &phase, has_output, &GateAnswer::Approved)
                        .map_err(sentence),
                    reject: answer_gate_enabled(
                        step,
                        &phase,
                        has_output,
                        &GateAnswer::Rejected {
                            note: String::new(),
                        },
                    )
                    .map_err(sentence),
                    retry: retry_admitted(run, item.status, steps, step, &phase).map_err(sentence),
                    promote: promote_enabled(
                        run,
                        item.status,
                        steps,
                        step,
                        &phase,
                        !live.is_empty(),
                    )
                    .map_err(sentence),
                    accept: accept_enabled(steps, step, &phase, has_output, live.contains(step.id))
                        .map_err(sentence),
                    select,
                    open,
                },
            );
        }
    }
    actions.unblock = match unblock_refusal {
        Some(refusal) => Err(refusal),
        None => unblock_enabled(item, &active).map(drop).map_err(sentence),
    };
    actions
}

/// Every step verdict of a run whose snapshot (or phase) could not be read: that sentence.
fn refused_step(refusal: &str) -> StepActions {
    let refused = || Err(refusal.to_owned());
    StepActions {
        approve: refused(),
        reject: refused(),
        retry: refused(),
        promote: refused(),
        accept: refused(),
        select: refused(),
        open: Err(refusal.to_owned()),
    }
}

/// The step `step` is not a fan-out candidate, so it has nothing to select.
fn not_a_candidate(step: StepId) -> String {
    format!("step {step} is not a fan-out candidate")
}

/// The newest head of `kind` produced by `step` — the engine's `output_of` rule — or the sentence
/// that says there is none.
fn newest_output(heads: &[DocumentHead], kind: &str, step: StepId) -> Result<DocumentId, String> {
    heads
        .iter()
        .filter(|head| head.kind == kind && head.produced_by_step_id == Some(step))
        .max_by_key(|head| head.version)
        .map(|head| head.id)
        .ok_or_else(|| format!("step {step} produced no `{kind}` document"))
}

/// Plan D155: `GraphSource` is `htui-orch`'s and `Backend` is `htui-store`'s, so `impl GraphSource
/// for Backend` here is E0117 (proven: plan Verified claims). A local newtype is the answer, and it
/// keeps invariant 10 (the orchestrator never names `htui-store`).
///
/// Every read delegates to the `Backend`-inherent read of the same name; `agent` filters
/// [`Backend::agents`] exactly as the `MemStore` implementation in `htui-orch`'s fake does.
#[derive(Debug, Clone)]
pub struct BackendGraphs(pub Backend);

impl GraphSource for BackendGraphs {
    async fn resolve_graph(&self, item: ItemId) -> StoreResult<Option<ResolvedGraph>> {
        self.0.resolve_graph(item).await
    }

    async fn phase_agents(&self, phase: PhaseId) -> StoreResult<Vec<PhaseAgent>> {
        self.0.phase_agents(phase).await
    }

    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> StoreResult<Option<PromptTemplate>> {
        self.0.prompt_template(project, name, version).await
    }

    async fn agent(&self, id: AgentId) -> StoreResult<Option<Agent>> {
        Ok(self
            .0
            .agents()
            .await?
            .into_iter()
            .map(|summary| summary.agent)
            .find(|agent| agent.id == id))
    }

    async fn agent_boxes(&self, box_id: BoxId) -> StoreResult<Vec<AgentBox>> {
        self.0.agent_boxes(box_id).await
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use htui_core::fixtures::{demo_at, ids};
    use std::collections::BTreeSet;

    use htui_core::model::{Agent, AgentBox, AgentId, Billing, RunId, RunMode, StepId, Transport};
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
    use htui_orch::{Command, GateAnswer, GraphSource};
    use htui_store::Backend;
    use serde_json::json;

    use super::{BackendGraphs, FrameKind, ORCH_NAMES, OrchRequest};
    use crate::store_worker::{self, StoreReply, StoreRequest};

    /// The scripted registry row every walk test runs on (blueprint F-O): an `acp` row, because
    /// the fixture graphs gate every phase and stage 1's inline-approval interlock skips a `cli`
    /// row at a gated phase; the factory reaches the fake by row data alone.
    fn scripted_row(id: AgentId) -> Agent {
        Agent {
            id,
            name: "scripted".to_owned(),
            transport: Transport::Acp,
            billing: Billing::Subscription,
            models: Vec::new(),
            default_model: Some("sonnet".to_owned()),
            launch: json!({ "command": "unused", "args": [] }),
            settings: json!({}),
            enabled: true,
            created_at: demo_at(0, 0),
            updated_at: demo_at(0, 0),
        }
    }

    /// The scripted agent's `agent_box` on the demo box, probed ready (rung 3, plan D62).
    fn ready_on_box(agent_id: AgentId, at: DateTime<Utc>) -> AgentBox {
        AgentBox {
            agent_id,
            box_id: ids::BOX,
            enabled: true,
            version: Some("0.0.0-fake".to_owned()),
            path: None,
            probed_at: Some(at),
            quota: None,
            quota_at: None,
            updated_at: at,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        }
    }

    /// Blueprint F-O: the demo with every fixture agent disabled, one scripted agent and its
    /// `agent_box` on the demo box, so rung 3 of the candidate chain names exactly it.
    async fn seeded_store() -> (MemStore, AgentId) {
        let store = MemStore::demo();
        for summary in store.agents().await.expect("the fixture's agents") {
            let mut row = summary.agent;
            row.enabled = false;
            store.upsert_agent(&row).await.expect("the row is disabled");
        }
        let agent = AgentId::new();
        store
            .upsert_agent(&scripted_row(agent))
            .await
            .expect("the scripted row lands");
        store
            .upsert_agent_box(&ready_on_box(agent, demo_at(0, 0)))
            .await
            .expect("the agent_box row lands");
        (store, agent)
    }

    /// Plan D155: the five trait reads are the inherent reads of the same name.
    #[tokio::test]
    async fn backend_graphs_delegates_each_read() {
        let (store, agent) = seeded_store().await;
        let backend = Backend::memory(store);
        let graphs = BackendGraphs(backend.clone());

        let resolved = GraphSource::resolve_graph(&graphs, ids::HTUI_ANA_2)
            .await
            .expect("the read answers");
        assert_eq!(
            resolved,
            backend
                .resolve_graph(ids::HTUI_ANA_2)
                .await
                .expect("the read answers")
        );
        let resolved = resolved.expect("the demo item has a graph");
        let phase = resolved.phases.first().expect("the graph has a phase");

        assert_eq!(
            GraphSource::phase_agents(&graphs, phase.phase.id)
                .await
                .expect("the read answers"),
            backend
                .phase_agents(phase.phase.id)
                .await
                .expect("the read answers")
        );
        assert_eq!(
            GraphSource::prompt_template(&graphs, ids::PROJECT_HTUI, &phase.phase.name, None)
                .await
                .expect("the read answers"),
            backend
                .prompt_template(ids::PROJECT_HTUI, &phase.phase.name, None)
                .await
                .expect("the read answers")
        );
        let row = GraphSource::agent(&graphs, agent)
            .await
            .expect("the read answers")
            .expect("the scripted row is registered");
        assert_eq!(row.name, "scripted");
        assert_eq!(
            GraphSource::agent(&graphs, AgentId::new())
                .await
                .expect("the read answers"),
            None,
            "an unknown id is no row"
        );
        let boxes = GraphSource::agent_boxes(&graphs, ids::BOX)
            .await
            .expect("the read answers");
        assert_eq!(
            boxes,
            backend
                .agent_boxes(ids::BOX)
                .await
                .expect("the read answers")
        );
        assert!(boxes.iter().any(|row| row.agent_id == agent));
    }

    /// One request per entry of `ORCH_NAMES`, in its order.
    fn every_orch_request() -> Vec<OrchRequest> {
        let (run, step) = (RunId::new(), StepId::new());
        vec![
            OrchRequest::Command(Command::StartRun {
                item: ids::HTUI_ANA_2,
                mode: RunMode::Manual,
                repo_scope: None,
            }),
            OrchRequest::Command(Command::AnswerGate {
                run,
                step,
                answer: GateAnswer::Approved,
            }),
            OrchRequest::Command(Command::RetryStep { run, step }),
            OrchRequest::Command(Command::CancelRun { run }),
            OrchRequest::Command(Command::SelectFanout {
                run,
                position: 0,
                attempt: 1,
                winner: step,
            }),
            OrchRequest::Command(Command::PromoteStep {
                run,
                step,
                chat_open: false,
            }),
            OrchRequest::Command(Command::AcceptArtifact {
                run,
                step,
                chat_live: false,
            }),
            OrchRequest::Command(Command::Unblock {
                item: ids::HTUI_ANA_2,
            }),
            OrchRequest::Command(Command::CloseOut {
                item: ids::HTUI_ANA_2,
            }),
            OrchRequest::CloseOutPreview {
                item: ids::HTUI_ANA_2,
            },
            OrchRequest::Cleanup { run },
        ]
    }

    /// Blueprint D209, §12: eleven distinct names, each the `name()` of its request, none shared
    /// with another `StoreRequest`.
    #[test]
    fn orch_names_are_eleven_distinct_request_names() {
        let requests = every_orch_request();
        assert_eq!(requests.len(), ORCH_NAMES.len());
        for (request, name) in requests.into_iter().zip(ORCH_NAMES) {
            assert_eq!(StoreRequest::Orch(request).name(), name);
        }
        let distinct: BTreeSet<&str> = ORCH_NAMES.into_iter().collect();
        assert_eq!(distinct.len(), 11);
        for other in [
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
            StoreRequest::Document(htui_core::model::DocumentId::new()),
            StoreRequest::RunActions(ids::HTUI_ANA_2),
            StoreRequest::Runs(ids::HTUI_ANA_2),
        ] {
            assert!(!distinct.contains(other.name()), "{}", other.name());
        }
    }

    /// Blueprint D183: with no runtime, the stream is acknowledged, the verdicts are read and the
    /// document is served; only a command is refused, by name.
    #[tokio::test]
    async fn a_try_serve_without_a_runtime_answers_the_stream_and_the_actions() {
        let backend = Backend::memory(MemStore::demo());

        let StoreReply::RunStream(frame) = store_worker::serve(
            &backend,
            &StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        )
        .await
        else {
            panic!("a subscription is acknowledged");
        };
        assert_eq!(frame.item, ids::HTUI_ANA_2);
        assert!(matches!(frame.kind, FrameKind::Subscribed));

        let StoreReply::RunActions(actions) =
            store_worker::serve(&backend, &StoreRequest::RunActions(ids::HTUI_ANA_2)).await
        else {
            panic!("the verdicts are read");
        };
        assert_eq!(actions.item, ids::HTUI_ANA_2);
        assert_eq!(actions.run, Ok(()), "an open item may start a run");

        let heads = backend
            .documents(ids::HTUI_FEAT_1)
            .await
            .expect("the demo documents");
        let head = heads.first().expect("the demo item has a document");
        let StoreReply::Document(document) =
            store_worker::serve(&backend, &StoreRequest::Document(head.id)).await
        else {
            panic!("the document is read");
        };
        assert_eq!(document.expect("the row exists").id, head.id);

        for request in every_orch_request() {
            let name = request.name();
            let reply = store_worker::serve(&backend, &StoreRequest::Orch(request)).await;
            assert!(
                matches!(&reply, StoreReply::Failed { request, message }
                    if *request == name && message == store_worker::NO_RUN_RUNTIME),
                "{reply:?}"
            );
        }
    }
}
