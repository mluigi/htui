//! MOD-4 milestone 6 against live Postgres (plan Task 9, blueprint §11): ANA-2 §12 criteria 17
//! and 20, ANA-5 criterion 18's persistence half, and plan D161's `blocked -> awaiting_approval`
//! edge, each driven through the production `run_worker::RunRuntime` over a `Backend::Online`.
//!
//! The fakes suite (`htui-orch`'s conformance over `MemStore`) and the harness suites
//! (`tests/chat.rs`, `tests/backlog.rs`) prove the same criteria against a memory store. What only
//! this file can prove is that the writes they rely on are the ones **Postgres** accepts: the
//! item-law edge T1 added is one that `PgStore`'s transition accepts, the close-out lands on the
//! server as one write (the summary and the `closed` item together, or neither), and a promoted
//! step's continued log is read back through the writer from the server, not from a mirror window
//! (H-9).
//!
//! Every case builds the same stack: a throwaway database with the demo world
//! (`testkit::demo_db`), a throwaway mirror (`CacheStore`), `Backend::Online` over the two, and a
//! [`Harness`] over that backend with a `RunRuntime` whose isolator and verifier are the fakes and
//! whose sessions are scripted. The candidate chain names exactly one agent, seeded through
//! `db.store` (blueprint F-O). The Harness answers `ConnectionInfo` at start, which over a
//! non-`Memory` backend reaches the keyring, so every case holds a `testkit::mock_keyring` guard.
//!
//! Each case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics
//! instead when `CI` is set (plan D13), like every other Postgres-backed suite.
#![cfg(feature = "testkit")]

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::run_worker::{OrchRequest, RunRuntime, StepAuthor};
use htui::store_worker::StoreRequest;
use htui::testkit::Harness;
use htui::ui::tabs::ChatTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::driver::{AgentDriver, DriverCaps};
use htui_agent::event::{DoneEvent, DriverEvent, StopReason, TextChunk, UsageEvent};
use htui_agent::fake::{FakeAdapter, FakeDriver};
use htui_agent::registry::{DriverFactory, TransportBuilder};
use htui_core::fixtures::ids;
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, DocumentHead, DocumentId, EventKind, Item, ItemId,
    NewDocument, NewRepo, RepoId, Run, RunId, RunMode, RunStatus, RunStep, SessionEvent,
    SnapshotPhase, Status, StepId, StepStatus, Transport, UsageTotals,
};
use htui_core::store::{ReadStore as _, WriteStore as _};
use htui_orch::fake::{FakeIsolator, FakeVerifier};
use htui_orch::{Command, GateAnswer};
use htui_store::{Backend, CacheStore, PgStore, testkit};
use serde_json::{Value, json};
use sqlx::Row as _;
use sqlx::postgres::PgPool;

/// How long a case waits for the rows a chat writes before it calls the chat stuck.
const PATIENCE: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------------------------
// The stack
// ---------------------------------------------------------------------------------------------

/// Blueprint D203's author: one document of the phase's `output_kind` per step, so every gate a
/// walk parks at has the output an approval needs. The body carries no front matter, so a
/// `review` settles `ok` and parks for the human (plan D10), which is what the escalation case
/// rejects.
#[derive(Debug)]
struct OutputAuthor;

impl StepAuthor for OutputAuthor {
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument> {
        Some(NewDocument {
            id: DocumentId::new(),
            item_id: item,
            kind: phase.output_kind.clone(),
            title: format!("{} (attempt {})", phase.output_kind, step.attempt),
            body: "authored".to_owned(),
            produced_by_step_id: Some(step.id),
            created_by: ids::USER,
            created_at: Utc::now(),
        })
    }
}

/// What every walk session reports before it ends its one turn: a nonzero spend, so the promoted
/// step's `run_step.usage` has a pre-promotion sum to carry forward.
fn walk_usage() -> UsageEvent {
    UsageEvent {
        input_tokens: Some(100),
        output_tokens: Some(10),
        cost_micros: Some(1_000),
        ..UsageEvent::default()
    }
}

/// What each turn of the promoted chat reports.
fn chat_usage() -> UsageEvent {
    UsageEvent {
        input_tokens: Some(7),
        output_tokens: Some(3),
        cost_micros: Some(50),
        ..UsageEvent::default()
    }
}

/// The walk's transport: every session plays one turn — a usage report, then `done`.
#[derive(Debug)]
struct Walks;

impl TransportBuilder for Walks {
    fn build(
        &self,
        agent: &Agent,
        _on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>, htui_agent::error::DriverError> {
        Ok(Box::new(FakeDriver::new(
            agent.name.clone(),
            caps,
            Script::one_turn(vec![
                ScriptEvent::Emit(DriverEvent::Usage(walk_usage())),
                done(),
            ]),
        )))
    }
}

/// Lets the chat runtime reach the adapter a case loaded its script into.
#[derive(Debug)]
struct SharedAdapter(Arc<FakeAdapter>);

impl TransportBuilder for SharedAdapter {
    fn build(
        &self,
        agent: &Agent,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>, htui_agent::error::DriverError> {
        self.0.build(agent, on_box, caps)
    }
}

fn done() -> ScriptEvent {
    ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
        stop_reason: StopReason::EndTurn,
    }))
}

fn chunk(text: &str) -> ScriptEvent {
    ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
        text: text.to_owned(),
        message_id: Some("m1".to_owned()),
    }))
}

/// One chat turn: some text, a usage report, the end of the turn.
fn chat_turn(text: &str) -> Vec<ScriptEvent> {
    vec![
        chunk(text),
        ScriptEvent::Emit(DriverEvent::Usage(chat_usage())),
        done(),
    ]
}

/// Blueprint F-O on the server: the demo's agents disabled, one scripted `acp` row probed ready on
/// this box (the candidate chain's rung 3), and a primary repo for the demo project, which is the
/// scope a default `StartRun` resolves to and the tree a promoted step chats in.
async fn seed(store: &PgStore) {
    for summary in store.agents().await.expect("the fixture's agents") {
        let mut row = summary.agent;
        row.enabled = false;
        store.upsert_agent(&row).await.expect("the row is disabled");
    }
    let agent_id = AgentId::new();
    let at = Utc::now();
    store
        .upsert_agent(&Agent {
            id: agent_id,
            name: "scripted".to_owned(),
            transport: Transport::Acp,
            billing: Billing::Subscription,
            models: Vec::new(),
            default_model: Some("sonnet".to_owned()),
            launch: json!({ "command": "unused", "args": [] }),
            settings: json!({}),
            enabled: true,
            created_at: at,
            updated_at: at,
        })
        .await
        .expect("the scripted row lands");
    store
        .upsert_agent_box(&AgentBox {
            agent_id,
            box_id: store.this_box(),
            enabled: true,
            version: Some("0.0.0-fake".to_owned()),
            path: None,
            probed_at: Some(at),
            quota: None,
            quota_at: None,
            updated_at: at,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        })
        .await
        .expect("the agent_box row lands");
    store
        .create_repo(NewRepo {
            id: RepoId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "htui".to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: true,
        })
        .await
        .expect("the demo project has no repo yet");
}

/// The run runtime over the fakes, whose sessions play [`Walks`].
fn run_runtime() -> RunRuntime {
    let mut factory = DriverFactory::new();
    factory.register("acp", Box::new(Walks));
    RunRuntime::with_parts(
        Arc::new(FakeIsolator::new()),
        Arc::new(FakeVerifier::new()),
        factory,
    )
    .with_author(Arc::new(OutputAuthor))
}

/// Everything one case holds: the database, the mirror (and the directory it lives in), the
/// keyring guard and the shell.
struct Stack {
    db: testkit::TestDb,
    _root: tempfile::TempDir,
    cache: CacheStore,
    _keyring: testkit::KeyringGuard,
    harness: Harness,
}

impl Stack {
    /// The stack over a seeded demo database, or `None` (after `testkit::SKIP`) without a server.
    ///
    /// `chat` is the promoted chat's script; `None` builds a shell with no chat runtime and no
    /// Chat tab, which the close-out and unblock cases need no more of.
    async fn new(chat: Option<Script>) -> Option<Self> {
        let db = testkit::demo_db().await?;
        seed(&db.store).await;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), "runs-pg", PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let keyring = testkit::mock_keyring().await;
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        };
        let mut harness = Harness::over_backend(backend).with_run_runtime(run_runtime());
        if let Some(chat) = chat {
            let adapter = Arc::new(FakeAdapter::new());
            adapter.load(chat);
            let mut factory = DriverFactory::new();
            factory.register("acp", Box::new(SharedAdapter(adapter)));
            harness = harness
                .with_tab(Box::new(ChatTab::new()))
                .with_replay_tab(ChatTab::ID)
                .with_agent_runtime(AgentRuntime::new(factory).with_grace(Duration::ZERO));
        }
        harness.drive().await;
        assert_eq!(
            harness.app().top_bar.store,
            "online",
            "the shell runs over the Postgres backend, not a memory one"
        );
        Some(Self {
            db,
            _root: root,
            cache,
            _keyring: keyring,
            harness,
        })
    }

    /// Sends one orchestrator command from the shell and drives until its walk has rested.
    async fn command(&mut self, command: Command) {
        self.harness
            .app()
            .update(Action::Store(StoreRequest::Orch(OrchRequest::Command(
                command,
            ))));
        self.harness.drive().await;
    }

    /// The status line, taken: what the last refusal said, and `None` again afterwards.
    fn take_status(&mut self) -> Option<String> {
        self.harness.app().status.take()
    }

    async fn item(&self, id: ItemId) -> Item {
        self.db
            .store
            .item(id)
            .await
            .expect("the read answers")
            .expect("the item exists")
    }

    async fn run(&self, id: RunId) -> Run {
        self.db
            .store
            .run(id)
            .await
            .expect("the read answers")
            .expect("the run exists")
    }

    async fn steps(&self, run: RunId) -> Vec<RunStep> {
        self.db
            .store
            .run_steps(run)
            .await
            .expect("the read answers")
    }

    /// The latest step at `position` of `run`.
    async fn step_at(&self, run: RunId, position: i32) -> RunStep {
        self.steps(run)
            .await
            .into_iter()
            .filter(|step| step.position == position)
            .max_by_key(|step| step.attempt)
            .expect("a step at the position")
    }

    /// The one step of `run` a gate is waiting on.
    async fn parked_step(&self, run: RunId) -> RunStep {
        let parked: Vec<RunStep> = self
            .steps(run)
            .await
            .into_iter()
            .filter(|step| step.status == StepStatus::AwaitingApproval)
            .collect();
        assert_eq!(parked.len(), 1, "one parked step: {parked:?}");
        parked.into_iter().next().expect("checked above")
    }

    /// The step's persisted log, in `seq` order.
    async fn log(&self, step: StepId) -> Vec<SessionEvent> {
        self.db
            .store
            .step_events(step)
            .await
            .expect("the log reads")
            .expect("the step has a log")
    }

    /// The item's document heads of kind `summary`.
    async fn summaries(&self, item: ItemId) -> Vec<DocumentHead> {
        self.documents(item)
            .await
            .into_iter()
            .filter(|head| head.kind == "summary")
            .collect()
    }

    async fn documents(&self, item: ItemId) -> Vec<DocumentHead> {
        self.db
            .store
            .documents(item)
            .await
            .expect("the read answers")
    }

    async fn notes(&self, item: ItemId) -> Vec<String> {
        self.db
            .store
            .notes(item)
            .await
            .expect("the read answers")
            .into_iter()
            .map(|note| note.body)
            .collect()
    }

    /// `StartRun` on `item` from the shell; the run it created, which the walk parked.
    async fn start(&mut self, item: ItemId) -> RunId {
        let before: Vec<RunId> = self.run_ids(item).await;
        self.command(Command::StartRun {
            item,
            mode: RunMode::Manual,
            repo_scope: None,
        })
        .await;
        assert_eq!(self.take_status(), None, "the start was not refused");
        let created: Vec<RunId> = self
            .run_ids(item)
            .await
            .into_iter()
            .filter(|run| !before.contains(run))
            .collect();
        assert_eq!(created.len(), 1, "one run created: {created:?}");
        created[0]
    }

    async fn run_ids(&self, item: ItemId) -> Vec<RunId> {
        self.db
            .store
            .runs(item)
            .await
            .expect("the read answers")
            .into_iter()
            .map(|summary| summary.id)
            .collect()
    }

    /// Answers the parked gate of `run`, and asserts nothing was refused.
    async fn answer(&mut self, run: RunId, answer: GateAnswer) -> RunStep {
        let step = self.parked_step(run).await;
        self.command(Command::AnswerGate {
            run,
            step: step.id,
            answer,
        })
        .await;
        assert_eq!(
            self.take_status(),
            None,
            "the answer on `{}` was not refused",
            step.phase_name
        );
        step
    }

    /// Drives the shell, with a short sleep between drives, until `done` holds.
    ///
    /// The Harness polls a chat future once per round and a round ends when nothing progressed. A
    /// chat recording into Postgres is `Pending` on the server between polls, which a single
    /// `drive` cannot tell from a chat waiting on the user (`testkit.rs`'s own note on
    /// `Writer::Buffered`), so this waits on the rows instead.
    async fn drive_until<F, Fut>(&mut self, what: &str, mut done: F)
    where
        F: FnMut(PgStore) -> Fut,
        Fut: Future<Output = bool>,
    {
        let deadline = Instant::now() + PATIENCE;
        loop {
            self.harness.drive().await;
            if done(self.db.store.clone()).await {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what} did not happen within {PATIENCE:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Closes the mirror and drops the database. Every case calls this on its last line.
    async fn finish(self) {
        let Self { db, cache, .. } = self;
        cache.close().await;
        db.drop_db().await;
    }
}

/// `run_step.usage`, which no `ReadStore` method returns (`chat_usage_pg.rs`'s runtime-checked
/// form: a `query!` here would need a `cargo sqlx prepare` pass `.sqlx/` does not own).
async fn step_usage(pool: &PgPool, step: StepId) -> Option<Value> {
    sqlx::query("SELECT usage FROM run_step WHERE id = $1")
        .bind(step.as_uuid())
        .fetch_one(pool)
        .await
        .expect("read run_step")
        .get::<Option<Value>, _>("usage")
}

/// `COUNT(*)` of `run` rows with `kind = 'chat'`.
async fn chat_runs(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM run WHERE kind = 'chat'")
        .fetch_one(pool)
        .await
        .expect("count chat runs")
}

/// Types `text` into the composer and submits it, as `tests/chat.rs` does.
fn compose(harness: &mut Harness, text: &str) {
    harness.key("i");
    for ch in text.chars() {
        if ch == ' ' {
            harness.key("space");
        } else {
            harness.key(&ch.to_string());
        }
    }
    harness.key("enter");
}

/// `ids::HTUI_FEAT_3` is seeded `queued` under a live `RUN_2`, and `create_run` moves an item
/// `open | failed -> queued` only; cancelling `RUN_2` moves it back to `open` (the conformance
/// suite's `free_feat_3`).
async fn free_feat_3(store: &PgStore) {
    store
        .finish_run(ids::RUN_2, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("the seeded run is queued and cancellable");
}

// ---------------------------------------------------------------------------------------------
// Criterion 17 and ANA-5 criterion 18
// ---------------------------------------------------------------------------------------------

/// ANA-2 §12 criterion 17 and ANA-5 criterion 18 on Postgres: a parked graph step promoted to a
/// chat keeps its id, the chat's messages land as `follow_up` rows at the step's next turns on
/// the **same** log, `promoted_at` is set, no `run(kind = 'chat')` row is minted, the step's
/// `prompt_digest` is the walk's prompt's, and `run_step.usage` is the pre-promotion sum plus the
/// chat's (the continuing recorder is seeded from the tail it read through the writer, H-9).
#[tokio::test(flavor = "multi_thread")]
async fn a_promoted_step_continues_its_own_log_on_postgres() {
    let Some(mut stack) = Stack::new(Some(Script::turns(vec![
        chat_turn("Picking the step back up."),
        chat_turn("Done as asked."),
    ])))
    .await
    else {
        return;
    };
    let item = ids::HTUI_ANA_2;
    let run = stack.start(item).await;
    assert_eq!(stack.run(run).await.status, RunStatus::AwaitingApproval);
    let step = stack.step_at(run, 0).await;
    assert_eq!(
        step.status,
        StepStatus::AwaitingApproval,
        "the first phase gates"
    );
    assert!(step.promoted_at.is_none());
    assert!(
        step.prompt_digest.is_some(),
        "the walk's prompt wrote the digest"
    );

    let walked = stack.log(step.id).await;
    let last_seq = walked.iter().map(|row| row.seq).max().expect("walk rows");
    let last_turn = walked.iter().map(|row| row.turn).max().expect("walk rows");
    let walked_usage = step_usage(&stack.db.pool, step.id).await;
    assert_eq!(
        walked_usage,
        Some(UsageTotals::from_rows(&walked).to_value()),
        "before the promotion `run_step.usage` is the walk's own sum"
    );
    let chat_runs_before = chat_runs(&stack.db.pool).await;

    // The Runs pane's `p`, as the shell receives it (D165).
    stack
        .harness
        .app()
        .update(Action::Promote { run, step: step.id });
    stack
        .drive_until("the handoff opening's answer", |store| async move {
            store
                .step_events(step.id)
                .await
                .ok()
                .flatten()
                .is_some_and(|log| {
                    log.iter().any(|row| {
                        row.kind == EventKind::AssistantText && row.turn == last_turn + 1
                    })
                })
        })
        .await;
    assert_eq!(stack.take_status(), None, "nothing was refused");
    assert_eq!(
        stack.harness.chat_steps(),
        vec![step.id],
        "the chat's session is the promoted step's"
    );
    assert_eq!(
        chat_runs(&stack.db.pool).await,
        chat_runs_before,
        "a promotion mints no `run(kind = 'chat')` (blueprint D205)"
    );

    compose(&mut stack.harness, "tighten the summary");
    stack
        .drive_until("the composed message's answer", |store| async move {
            store
                .step_events(step.id)
                .await
                .ok()
                .flatten()
                .is_some_and(|log| {
                    log.iter().any(|row| {
                        row.kind == EventKind::AssistantText && row.turn == last_turn + 2
                    })
                })
        })
        .await;
    // `Esc Esc` ends the session only (plan D165); `drive_to_end` then awaits its last writes.
    stack.harness.key("esc");
    stack.harness.key("esc");
    stack.harness.drive_to_end().await;

    let log = stack.log(step.id).await;
    assert!(
        log.iter().all(|row| row.run_step_id == step.id),
        "one log, one step"
    );
    let seqs: Vec<i32> = log.iter().map(|row| row.seq).collect();
    assert_eq!(
        seqs,
        (0..i32::try_from(log.len()).expect("a short log")).collect::<Vec<_>>(),
        "the log stays gapless across the promotion"
    );
    let prompts: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::Prompt)
        .collect();
    assert_eq!(prompts.len(), 1, "one prompt row: the walk's");
    let follow_ups: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::FollowUp)
        .collect();
    assert_eq!(
        follow_ups.iter().map(|row| row.turn).collect::<Vec<_>>(),
        vec![last_turn + 1, last_turn + 2],
        "the handoff opening, then the composed message, each opening the step's next turn"
    );
    assert_eq!(
        follow_ups[0].seq,
        last_seq + 1,
        "the opening is the row right after the walk's last (ANA-5 criterion 18)"
    );

    let after = stack.step_at(run, 0).await;
    assert_eq!(after.id, step.id, "the step keeps its id");
    assert!(after.promoted_at.is_some(), "`promoted_at` is set");
    assert_eq!(
        after.prompt_digest, step.prompt_digest,
        "the step's digest is the walk's prompt's"
    );
    assert_eq!(after.status, StepStatus::AwaitingApproval);
    assert_eq!(stack.run(run).await.status, RunStatus::AwaitingApproval);
    assert_eq!(
        chat_runs(&stack.db.pool).await,
        chat_runs_before,
        "and none was minted by the time the chat ended"
    );
    assert_eq!(
        stack.run_ids(item).await.len(),
        1,
        "the item still has its one graph run"
    );

    let expected = UsageTotals {
        input_tokens: Some(100 + 7 + 7),
        output_tokens: Some(10 + 3 + 3),
        cost_micros: Some(1_000 + 50 + 50),
        ..UsageTotals::default()
    };
    let persisted = step_usage(&stack.db.pool, step.id).await;
    assert_eq!(
        persisted,
        Some(expected.to_value()),
        "`run_step.usage` is the pre-promotion sum plus the chat's two turns"
    );
    assert_eq!(
        persisted,
        Some(UsageTotals::from_rows(&log).to_value()),
        "which is the sum of every `usage` row on the step's one log"
    );

    stack.finish().await;
}

// ---------------------------------------------------------------------------------------------
// Criterion 20
// ---------------------------------------------------------------------------------------------

/// ANA-2 §12 criterion 20 on Postgres (plan D167): while a run of the item is live the close-out
/// is refused and writes nothing; once the run is done it writes exactly one `summary` document
/// and closes the item with `closed_at` set, together.
#[tokio::test(flavor = "multi_thread")]
async fn close_out_is_one_transaction_on_postgres() {
    let Some(mut stack) = Stack::new(None).await else {
        return;
    };
    let item = ids::HTUI_FEAT_3;
    free_feat_3(&stack.db.store).await;
    assert!(
        stack.summaries(item).await.is_empty(),
        "the fixture holds no summary"
    );
    let run = stack.start(item).await;
    assert_eq!(stack.run(run).await.status, RunStatus::AwaitingApproval);

    let documents_before = stack.documents(item).await;
    stack.command(Command::CloseOut { item }).await;
    let refused = stack.take_status().expect("the close-out was refused");
    assert!(
        refused.starts_with("close_out: ") && refused.contains(&run.to_string()),
        "the refusal names the live run: {refused}"
    );
    assert_eq!(
        stack.documents(item).await,
        documents_before,
        "a refused close-out writes no document"
    );
    let held = stack.item(item).await;
    assert_eq!(held.status, Status::AwaitingApproval, "and moves nothing");
    assert!(held.closed_at.is_none());

    for _ in 0..4 {
        stack.answer(run, GateAnswer::Approved).await;
    }
    assert_eq!(
        stack.run(run).await.status,
        RunStatus::Done,
        "four gates approved"
    );
    assert_eq!(stack.item(item).await.status, Status::Done);

    let documents_before = stack.documents(item).await.len();
    stack.command(Command::CloseOut { item }).await;
    assert_eq!(stack.take_status(), None, "the close-out was accepted");
    let summaries = stack.summaries(item).await;
    assert_eq!(summaries.len(), 1, "exactly one summary: {summaries:?}");
    assert_eq!(
        stack.documents(item).await.len(),
        documents_before + 1,
        "and no other document"
    );
    let summary = stack
        .db
        .store
        .document(summaries[0].id)
        .await
        .expect("the read answers")
        .expect("the summary was written");
    assert_eq!(
        (
            summary.kind.as_str(),
            summary.version,
            summary.produced_by_step_id
        ),
        ("summary", 1, None)
    );
    let closed = stack.item(item).await;
    assert_eq!(closed.status, Status::Closed);
    assert!(closed.closed_at.is_some(), "`closed_at` is set");

    stack.finish().await;
}

// ---------------------------------------------------------------------------------------------
// Plan D161's edge
// ---------------------------------------------------------------------------------------------

/// Plan D161 case 2 on Postgres: the review loop escalates — the item `blocked`, the run parked —
/// and `Unblock` lets the item follow its run to `awaiting_approval`, T1's new item-law edge. The
/// run's own verbs then reach it again: the review is promoted.
#[tokio::test(flavor = "multi_thread")]
async fn unblock_follows_an_escalated_run_on_postgres() {
    let Some(mut stack) = Stack::new(None).await else {
        return;
    };
    let item = ids::HTUI_FEAT_3;
    free_feat_3(&stack.db.store).await;
    let run = stack.start(item).await;

    // `prd`, `plan`, `implement` approved; `review` rejected; the second `implement` approved;
    // the second `review` rejected, which the loop cannot make progress past.
    let mut answered = Vec::new();
    for _ in 0..8 {
        if stack.item(item).await.status == Status::Blocked {
            break;
        }
        let parked = stack.parked_step(run).await;
        let answer = if parked.phase_name == "review" {
            GateAnswer::Rejected {
                note: "no tests".to_owned(),
            }
        } else {
            GateAnswer::Approved
        };
        let step = stack.answer(run, answer).await;
        answered.push((step.phase_name, step.attempt));
    }
    assert_eq!(
        answered,
        [
            ("prd".to_owned(), 1),
            ("plan".to_owned(), 1),
            ("implement".to_owned(), 1),
            ("review".to_owned(), 1),
            ("implement".to_owned(), 2),
            ("review".to_owned(), 2),
        ],
        "the walk the escalation needs"
    );
    assert_eq!(
        stack.item(item).await.status,
        Status::Blocked,
        "the loop escalated"
    );
    assert_eq!(stack.run(run).await.status, RunStatus::AwaitingApproval);
    let review = stack.step_at(run, 3).await;
    assert_eq!((review.status, review.attempt), (StepStatus::Failed, 2));

    stack.command(Command::Unblock { item }).await;
    assert_eq!(stack.take_status(), None, "the unblock was accepted");
    assert_eq!(
        stack.item(item).await.status,
        Status::AwaitingApproval,
        "plan D161's `blocked -> awaiting_approval` edge, accepted by the server"
    );
    assert_eq!(
        stack.run(run).await.status,
        RunStatus::AwaitingApproval,
        "the run is where the escalation parked it"
    );
    assert!(
        stack.notes(item).await.contains(&format!(
            "unblocked: follows run {run}, parked at `awaiting_approval`"
        )),
        "a human reads what was done"
    );

    // R-4: the run's own verbs reach it now. With no chat runtime in this shell the promotion is
    // answered after the engine's writes.
    stack
        .command(Command::PromoteStep {
            run,
            step: review.id,
            chat_open: false,
        })
        .await;
    assert_eq!(
        stack.take_status().as_deref(),
        Some("promote_step: promotion needs the chat runtime")
    );
    let promoted = stack.step_at(run, 3).await;
    assert_eq!(promoted.id, review.id);
    assert!(
        promoted.promoted_at.is_some(),
        "the escalated review was promoted"
    );

    stack.finish().await;
}
