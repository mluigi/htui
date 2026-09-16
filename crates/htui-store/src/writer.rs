//! An owned handle on a writable store (MOD-2 milestone 3, plan D26).
//!
//! [`Backend::writable`](crate::Backend::writable) hands out a `&PgStore`, which is the right
//! shape for a caller that writes and returns. A **recorder** is not that caller: it lives for a
//! whole chat session, inside a task the store worker spawned, and it is generic over
//! `S: WriteStore` — it needs something it can own.
//!
//! [`Writer`] is that something. Every arm is a cheap handle — `PgStore` is a pool handle,
//! `MemStore` is an `Arc`, [`BufferedWriter`] is a `CacheStore` handle and a path — so a `Writer`
//! is a clone of a handle, never a copy of a store.
//!
//! There are **three** arms, but since MOD-25 only two of them are ever constructed. Between
//! MOD-2 milestone 4 and MOD-25 [`Backend::writer`](crate::Backend::writer) answered `Some` on
//! [`Backend::Offline`](crate::Backend::Offline) too (plan D34): an offline chat recorded the same
//! rows to `<cache_dir>/pending/` as JSON lines, and the refresher uploaded them on the next
//! connection. That was a different sink, not a different recorder — the recorder is generic over
//! `S: WriteStore` and learns nothing about being offline, so every conformance case that passes
//! online passed offline with the same rows.
//!
//! **MOD-25 made `htui` online-only**: `Backend::writer()` answers `None` off the server and a
//! chat there is refused with [`DATABASE_UNREACHABLE`], so no backend hands out
//! [`Writer::Buffered`] any more. The arm and [`BufferedWriter`] are kept, compiling and `pub`,
//! for one release, so reversing MOD-25 is restoring one arm in `backend.rs`; the suites that
//! prove them construct [`BufferedWriter`] directly, and `upload_pending` still runs on every
//! refresh pass so buffers from earlier builds land. A later CLEAN item deletes all of it. What
//! has **not** changed at any point is the invariant that matters: nothing writes to Postgres
//! unless the backend is `Online`, and [`Backend::writable`](crate::Backend::writable) still
//! answers `None` off the server.
//!
//! `MemStore` is reachable here and is not through `writable`, deliberately: `--demo` and every
//! chat-tab snapshot run against it, and a seam only the production backend can exercise is a seam
//! no test covers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use htui_core::model::{
    Agent, AgentBox, AgentId, BoxId, ChatRunSpec, Document, DocumentHead, DocumentId, Item,
    ItemFilter, ItemId, ItemKind, ItemKindId, ItemKindPatch, ItemPatch, ItemSummary, LinkGraph,
    NewItem, NewItemKind, NewProject, NewRepo, NewStepGraph, NewWorkspace, Note, PhaseId,
    PhasePatch, Project, ProjectId, ProjectPatch, PromptScope, Repo, RepoBoxPath, RepoId,
    RepoPatch, RunId, RunStatus, RunSummary, Scope, SessionEvent, Status, StepGraph, StepGraphId,
    StepGraphPatch, StepGraphPhase, StepId, UpstreamEntry, Workspace, WorkspaceBoxPath,
    WorkspaceId, WorkspacePatch, WorkspaceProject,
};
use htui_core::prompt::settings::SettingKey;
use htui_core::store::{
    CasOutcome, DeleteReach, DeleteTarget, MemStore, ReadStore, Result, SettingRung, StoreError,
    StoredSetting, UpdateOutcome, WriteStore,
};
use serde_json::Value;

use crate::cache::CacheStore;
use crate::cache::pending::{append_pending, seal_pending};
use crate::pg::PgStore;

/// A writable store a caller can hold.
#[derive(Debug, Clone)]
pub enum Writer {
    /// In-process (`--demo`, tests).
    Memory(MemStore),
    /// Postgres.
    Online(PgStore),
    /// The offline sink (MOD-2 plan D34): reads from the mirror, writes to `<cache_dir>/pending/`.
    ///
    /// **Kept, but not constructed by any backend since MOD-25** — `htui` is online-only and a
    /// chat off the server is refused with [`DATABASE_UNREACHABLE`]. The arm stays for one
    /// release so the reversal is a one-arm change in `backend.rs`; the CLEAN item removes it.
    Buffered(BufferedWriter),
}

impl Writer {
    /// The backend label this writer belongs to, for logs and for the chat header (plan D42: the
    /// maintainer must be able to tell a recorded conversation from one that is only on this disk).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory",
            Self::Online(_) => "online",
            Self::Buffered(_) => "buffered",
        }
    }
}

/// The offline [`WriteStore`] (MOD-2 plan D34, D35): the rows an online chat sends to Postgres,
/// appended to `<cache_dir>/pending/<project>.<run>.jsonl.open` as JSON lines instead.
///
/// It writes what the buffer's line format can hold and refuses the rest. That format is
/// `session_event` columns only ([`crate::cache::pending`]), so:
///
/// - `append_events` is the whole point, and it needs the `(project, run)` pair the file name
///   carries. `start_chat_run` is where that pair arrives, so it **registers** the chat's step
///   rather than writing a row; a later event for a step nobody registered is a
///   [`StoreError::NotFound`], never a silent drop.
/// - `set_step_usage` is a no-op: `run_step.usage` has nowhere to go in the buffer, and
///   `upload_pending` recomputes it from the uploaded rows (plan D36) rather than losing it.
/// - `set_step_prompt` is a **refusal**, and the contrast with the line above is the point: no
///   upload can recompute `run_step.trim_record`, so a no-op would silently drop the audit row
///   `R-PRM-3` requires to be "recorded on the step" (MOD-2 milestone 9, blueprint E-8).
/// - `finish_chat_run` **seals** the buffer (risk `[H-1]`), which is a deliberate deviation from
///   D35's "a no-op that logs at debug": without it, a chat that outlives a reconnect is uploaded
///   mid-flight and its `run_step.usage` frozen at a partial sum.
/// - item and registry writes answer [`StoreError::Unreachable`]: they genuinely need the server.
///
/// Two consequences worth stating, both of a buffered chat that a build before MOD-25 started.
/// Such a chat is **not** in the mirror's `run` table until it is uploaded, so `active_runs` and
/// the top bar do not count it — the D42 header is the one place it shows. And every construction
/// of this writer yields a fresh, empty one, which was right because the runtime took exactly one
/// per chat and moved it into that chat's session task, so its `start_chat_run` and its
/// `append_events` shared one map. Since MOD-25 no [`Backend::writer`](crate::Backend::writer)
/// call constructs it at all: the type is kept for one release for the reversal, and the upload
/// side still lands whatever an earlier build buffered.
#[derive(Debug, Clone)]
pub struct BufferedWriter {
    /// Reads, and the directory the buffer lives under.
    cache: CacheStore,
    /// `cache.dir()`, copied once: the argument every `cache::pending` call takes.
    dir: PathBuf,
    /// Filled by `start_chat_run`, read by `append_events` and `finish_chat_run`. `Arc` because
    /// the recorder borrows a clone of the writer the session task owns, and the two must see one
    /// map.
    runs: Arc<Mutex<HashMap<StepId, (ProjectId, RunId)>>>,
}

impl BufferedWriter {
    /// A writer over this mirror's directory, with nothing registered yet.
    #[must_use]
    pub fn new(cache: CacheStore) -> Self {
        let dir = cache.dir().to_path_buf();
        Self {
            cache,
            dir,
            runs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// `<root>/cache/<fingerprint>`; the buffer files live in `dir().join("pending")`.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The `(project, run)` a step was registered under, or `None` — for tests and logs.
    #[must_use]
    pub fn run_of(&self, step: StepId) -> Option<(ProjectId, RunId)> {
        self.registrations().get(&step).copied()
    }

    /// A copy of the registration map.
    ///
    /// The lock is taken and released inside this call, so no caller can hold it across an
    /// `.await` — the crate rule — and the map holds a handful of id pairs per chat. A poisoned
    /// lock is read through rather than propagated: the only writer is `start_chat_run`, and a
    /// panic there costs the chat its registrations, which surfaces as the `NotFound` above
    /// instead of as a second panic inside a session task.
    fn registrations(&self) -> HashMap<StepId, (ProjectId, RunId)> {
        self.runs.lock().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |runs| runs.clone(),
        )
    }

    /// The `(project, run)` of `step`, or the [`StoreError::NotFound`] an unregistered step gets.
    fn resolve(
        registrations: &HashMap<StepId, (ProjectId, RunId)>,
        step: StepId,
    ) -> Result<(ProjectId, RunId)> {
        registrations
            .get(&step)
            .copied()
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })
    }
}

/// Plain delegation to the mirror: the recorder never reads, and the `WriteStore: ReadStore` bound
/// wants these seven anyway.
impl ReadStore for BufferedWriter {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        self.cache.items(scope, filter).await
    }

    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        self.cache.item(id).await
    }

    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        self.cache.links(id, hops).await
    }

    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        self.cache.documents(id).await
    }

    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        self.cache.notes(id).await
    }

    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        self.cache.runs(id).await
    }

    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        self.cache.step_events(step).await
    }

    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        self.cache.document(id).await
    }

    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        self.cache.documents_of_kinds(item, kinds).await
    }

    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        self.cache.upstream_summaries(id, hops, scope).await
    }

    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        self.cache.project(id).await
    }
}

impl WriteStore for BufferedWriter {
    async fn mint_item(&self, _new: NewItem) -> Result<Item> {
        Err(item_writes_need_the_server())
    }

    async fn update_item(
        &self,
        _id: ItemId,
        _expected_version: i32,
        _patch: ItemPatch,
    ) -> Result<UpdateOutcome> {
        Err(item_writes_need_the_server())
    }

    async fn transition(&self, _id: ItemId, _from: Status, _to: Status) -> Result<bool> {
        Err(item_writes_need_the_server())
    }

    /// Groups `events` by `run_step_id` and appends each group to that run's buffer file, in the
    /// order the caller gave them, answering how many lines were written across every file.
    ///
    /// **Every** group is resolved against the registration map *before* the first byte is
    /// written, so a batch naming one unregistered step writes nothing at all — the trait's
    /// "either every new row lands or none does", kept at the granularity the file can offer.
    ///
    /// The events are appended verbatim: scrubbing is the recorder's job and has already happened,
    /// and this crate does not look inside a payload.
    async fn append_events(&self, events: &[SessionEvent]) -> Result<usize> {
        let Some(first) = events.first() else {
            return Ok(0);
        };
        let registrations = self.registrations();

        // The common case by far: the recorder flushes one step's rows at a time, and this path
        // hands the caller's own slice to the appender rather than copying every payload.
        if events
            .iter()
            .all(|event| event.run_step_id == first.run_step_id)
        {
            let (project, run) = Self::resolve(&registrations, first.run_step_id)?;
            return append_pending(&self.dir, project, run, events).await;
        }

        // Groups in first-seen order, so two runs interleaved in one batch keep each file's rows
        // in the order they were recorded.
        let mut groups: Vec<(StepId, Vec<SessionEvent>)> = Vec::new();
        for event in events {
            match groups
                .iter_mut()
                .find(|(step, _)| *step == event.run_step_id)
            {
                Some((_, rows)) => rows.push(event.clone()),
                None => groups.push((event.run_step_id, vec![event.clone()])),
            }
        }
        let resolved = groups
            .into_iter()
            .map(|(step, rows)| Self::resolve(&registrations, step).map(|pair| (pair, rows)))
            .collect::<Result<Vec<_>>>()?;

        let mut written = 0usize;
        for ((project, run), rows) in resolved {
            written += append_pending(&self.dir, project, run, &rows).await?;
        }
        Ok(written)
    }

    /// A no-op (plan D35): the buffer's line format holds `session_event` columns only, so there
    /// is nowhere to put a `run_step` column.
    ///
    /// The number is not lost — `upload_pending` recomputes it from the uploaded rows with the
    /// same summing rule the recorder used (plan D36), which is what keeps criterion 7 true for a
    /// chat that happened to be offline.
    async fn set_step_usage(
        &self,
        step: StepId,
        _usage: Value,
        _prompt_digest: Option<String>,
    ) -> Result<()> {
        tracing::debug!(%step, "buffered: run_step.usage is recomputed at upload (D36)");
        Ok(())
    }

    /// Refused, not a no-op — which is the whole difference from
    /// [`set_step_usage`](BufferedWriter::set_step_usage) directly above (blueprint E-8).
    ///
    /// That one may be silent because `upload_pending` recomputes `run_step.usage` from the
    /// uploaded `session_event` rows (plan D36), so the figure is deferred rather than lost.
    /// Nothing can recompute a **trim record**: the buffer's line format holds `session_event`
    /// columns only, and the `prompt` event's payload carries an abridged `sections[]` that is a
    /// lossy projection of the record, not the record. A silent no-op would therefore leave a step
    /// with a `prompt` event and no audit row, which is exactly what `R-PRM-3`'s "recorded on the
    /// step" forbids.
    ///
    /// `R-STO-4` starts no graph run offline, so nothing in this milestone reaches this arm. When
    /// MOD-4 first does, the refusal is the signal that an offline graph step needs a design — not
    /// a stub that already silently discarded its audit.
    ///
    /// # Errors
    ///
    /// Always [`StoreError::Unreachable`] with [`PROMPT_ON_SERVER_ONLY`].
    async fn set_step_prompt(&self, step: StepId, _digest: &str, _trim: &Value) -> Result<()> {
        tracing::debug!(%step, "buffered: run_step.trim_record cannot be buffered (E-8)");
        Err(StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))
    }

    async fn upsert_agent(&self, _agent: &Agent) -> Result<()> {
        Err(registry_writes_need_the_server())
    }

    async fn upsert_agent_box(&self, _row: &AgentBox) -> Result<()> {
        Err(registry_writes_need_the_server())
    }

    /// Refused with the other registry writes (MOD-2 plan D68): the cache mirror holds no
    /// `agent_box` table at all (`cache_migrations/0002_agent_mirror.sql:9-11`), so there is
    /// nowhere to put a latch offline.
    ///
    /// The refusal is not a lost figure. An offline chat leaves the last server-side quota
    /// standing and buffers its `usage` rows, from which `upload_pending` re-derives the spend -
    /// the same trade `set_step_usage` makes above. The chat asks **before** the first `usage`
    /// row rather than discovering this on it (`agent_worker::quota_latch_for`).
    async fn set_agent_box_quota(
        &self,
        _agent_id: AgentId,
        _box_id: BoxId,
        _quota: Value,
        _quota_at: DateTime<Utc>,
    ) -> Result<()> {
        Err(registry_writes_need_the_server())
    }

    /// Registers the chat's step under its `(project, run)` pair and writes no row.
    ///
    /// The pair is what the buffer's file name carries, and `append_events` is the only thing that
    /// needs it; the `run` / `run_step` rows themselves are synthesised by `upload_pending` from
    /// that name and the events, which is what makes an uploaded chat converge with an online one.
    async fn start_chat_run(&self, chat: &ChatRunSpec) -> Result<()> {
        let pair = (chat.project_id, chat.run_id);
        match self.runs.lock() {
            Ok(mut runs) => {
                runs.insert(chat.step_id, pair);
            }
            Err(poisoned) => {
                poisoned.into_inner().insert(chat.step_id, pair);
            }
        }
        Ok(())
    }

    /// `[H-1]` Seals this chat's buffer so `upload_pending` can take it, and writes no row.
    ///
    /// This is the **deviation from D35**, which asked for a no-op: a buffer only becomes
    /// uploadable when its chat ends, otherwise the refresher's next pass takes a live chat's
    /// partial file, closes the `run` as `done` and freezes `run_step.usage` at a partial sum.
    /// `status` and `finished_at` are not used — the uploader derives both from the events, and
    /// the line format carries neither.
    ///
    /// The registration is deliberately **kept**: a flush that arrives after the seal appends to a
    /// fresh open buffer rather than losing its rows, and that one is sealed by the next
    /// `CacheStore::open` or by a later seal - under a sealed name of its own, never appended to
    /// the file the first seal produced.
    async fn finish_chat_run(
        &self,
        _run: RunId,
        step: StepId,
        _status: RunStatus,
        _finished_at: DateTime<Utc>,
    ) -> Result<()> {
        let (project, run) = Self::resolve(&self.registrations(), step)?;
        let sealed = seal_pending(&self.dir, project, run).await?;
        tracing::debug!(%step, sealed, "buffered: the chat buffer is closed");
        Ok(())
    }

    // ---- MOD-15 milestone 1: the hierarchy (plan D2) ---------------------------------------
    //
    // Thirty-one refusals through one helper, readers included. `htui` is online-only since
    // MOD-25, so a hierarchy write off the server is refused rather than buffered; and none of
    // these six tables is mirrored (`cache::MIRRORED_TABLES`), so offline a *read* of them is
    // unreachable in the literal sense too. One sentence covers both directions, which is why
    // this needs no constant of its own — see [`hierarchy_needs_the_server`].
    //
    // They are one-liners on purpose. A real buffered implementation would be a second
    // hierarchy with a second set of rules to keep in step, written to be deleted: CLEAN-2
    // removes `BufferedWriter` whole once MOD-25's decision has settled.

    async fn create_workspace(&self, _new: NewWorkspace) -> Result<Workspace> {
        Err(hierarchy_needs_the_server())
    }

    async fn update_workspace(
        &self,
        _id: WorkspaceId,
        _expected: DateTime<Utc>,
        _patch: WorkspacePatch,
    ) -> Result<CasOutcome<Workspace>> {
        Err(hierarchy_needs_the_server())
    }

    async fn workspace(&self, _id: WorkspaceId) -> Result<Option<Workspace>> {
        Err(hierarchy_needs_the_server())
    }

    async fn upsert_workspace_project(&self, _link: &WorkspaceProject) -> Result<()> {
        Err(hierarchy_needs_the_server())
    }

    async fn remove_workspace_project(
        &self,
        _workspace: WorkspaceId,
        _project: ProjectId,
    ) -> Result<()> {
        Err(hierarchy_needs_the_server())
    }

    async fn workspace_projects(&self, _workspace: WorkspaceId) -> Result<Vec<WorkspaceProject>> {
        Err(hierarchy_needs_the_server())
    }

    async fn upsert_workspace_box_path(&self, _path: &WorkspaceBoxPath) -> Result<()> {
        Err(hierarchy_needs_the_server())
    }

    async fn workspace_box_paths(&self, _workspace: WorkspaceId) -> Result<Vec<WorkspaceBoxPath>> {
        Err(hierarchy_needs_the_server())
    }

    async fn create_project(&self, _new: NewProject) -> Result<Project> {
        Err(hierarchy_needs_the_server())
    }

    async fn update_project(
        &self,
        _id: ProjectId,
        _expected: DateTime<Utc>,
        _patch: ProjectPatch,
    ) -> Result<CasOutcome<Project>> {
        Err(hierarchy_needs_the_server())
    }

    async fn create_repo(&self, _new: NewRepo) -> Result<Repo> {
        Err(hierarchy_needs_the_server())
    }

    async fn update_repo(
        &self,
        _id: RepoId,
        _expected: DateTime<Utc>,
        _patch: RepoPatch,
    ) -> Result<CasOutcome<Repo>> {
        Err(hierarchy_needs_the_server())
    }

    async fn repos(&self, _project: ProjectId) -> Result<Vec<Repo>> {
        Err(hierarchy_needs_the_server())
    }

    async fn upsert_repo_box_path(&self, _path: &RepoBoxPath) -> Result<()> {
        Err(hierarchy_needs_the_server())
    }

    async fn repo_box_paths(&self, _repo: RepoId) -> Result<Vec<RepoBoxPath>> {
        Err(hierarchy_needs_the_server())
    }

    async fn create_item_kind(&self, _new: NewItemKind) -> Result<ItemKind> {
        Err(hierarchy_needs_the_server())
    }

    async fn update_item_kind(
        &self,
        _id: ItemKindId,
        _expected: DateTime<Utc>,
        _patch: ItemKindPatch,
    ) -> Result<CasOutcome<ItemKind>> {
        Err(hierarchy_needs_the_server())
    }

    async fn item_kinds(&self, _project: ProjectId) -> Result<Vec<ItemKind>> {
        Err(hierarchy_needs_the_server())
    }

    async fn delete_item_kind(&self, _id: ItemKindId) -> Result<()> {
        Err(hierarchy_needs_the_server())
    }

    async fn create_step_graph(&self, _new: NewStepGraph) -> Result<StepGraph> {
        Err(hierarchy_needs_the_server())
    }

    async fn update_step_graph(
        &self,
        _id: StepGraphId,
        _expected: DateTime<Utc>,
        _patch: StepGraphPatch,
    ) -> Result<CasOutcome<StepGraph>> {
        Err(hierarchy_needs_the_server())
    }

    async fn step_graphs(&self, _project: ProjectId) -> Result<Vec<StepGraph>> {
        Err(hierarchy_needs_the_server())
    }

    async fn create_phase(&self, _phase: &StepGraphPhase) -> Result<StepGraphPhase> {
        Err(hierarchy_needs_the_server())
    }

    async fn update_phase(
        &self,
        _id: PhaseId,
        _expected: DateTime<Utc>,
        _patch: PhasePatch,
    ) -> Result<CasOutcome<StepGraphPhase>> {
        Err(hierarchy_needs_the_server())
    }

    async fn phases(&self, _graph: StepGraphId) -> Result<Vec<StepGraphPhase>> {
        Err(hierarchy_needs_the_server())
    }

    async fn set_setting(
        &self,
        _rung: SettingRung,
        _key: SettingKey,
        _value: Value,
        _expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<StoredSetting>> {
        Err(hierarchy_needs_the_server())
    }

    async fn clear_setting(
        &self,
        _rung: SettingRung,
        _key: SettingKey,
        _expected: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>> {
        Err(hierarchy_needs_the_server())
    }

    async fn setting(&self, _rung: SettingRung, _key: SettingKey) -> Result<Option<StoredSetting>> {
        Err(hierarchy_needs_the_server())
    }

    async fn delete_reach(&self, _target: DeleteTarget) -> Result<Option<DeleteReach>> {
        Err(hierarchy_needs_the_server())
    }

    async fn delete_workspace(&self, _id: WorkspaceId) -> Result<DeleteReach> {
        Err(hierarchy_needs_the_server())
    }

    async fn delete_project(&self, _id: ProjectId) -> Result<DeleteReach> {
        Err(hierarchy_needs_the_server())
    }
}

/// D35's refusal for the three item writes: offline item editing is MOD-13's question.
fn item_writes_need_the_server() -> StoreError {
    StoreError::Unreachable(
        "item writes need the server; offline item editing is MOD-13's".to_owned(),
    )
}

/// D35's refusal for the registry writes, and MOD-2 D52's for a probe that has no server to
/// write its snapshot to: the same sentence in both places, on purpose. It is also what an
/// offline chat logs when it declines to latch a quota (MOD-2 plan D68).
///
/// A probe costs process spawns, so the caller checks this **before** it spawns anything rather
/// than discovering the refusal on the write. Writing a probe result somewhere local is the
/// local-only store's job (MOD-17), not this seam's.
pub const REGISTRY_ON_SERVER_ONLY: &str = "the agent registry is written on the server only";

/// D35's refusal for the two registry writes.
fn registry_writes_need_the_server() -> StoreError {
    StoreError::Unreachable(REGISTRY_ON_SERVER_ONLY.to_owned())
}

/// The one sentence the whole prompt path answers with off the server (MOD-2 plan D109,
/// blueprint E-8).
///
/// It names **two** facts because they are the same fact from either end. Reading: four of the
/// assembler's inputs — `prompt_template`, `skill`, `skill_version`, `skill_binding`, `box_tool` —
/// have no cache mirror, so a preview cannot be assembled offline at all. Writing:
/// `run_step.trim_record` has nowhere to go in the pending buffer, whose line format is
/// `session_event` columns only, and unlike `run_step.usage` nothing can recompute it at upload.
///
/// One constant, so a user who meets the refusal from the preview and from a step's audit row
/// reads the same sentence and does not have to decide whether they are two problems.
pub const PROMPT_ON_SERVER_ONLY: &str = "the prompt path needs the server: templates, skills and box tools are not mirrored, and a \
     step's trim record has nowhere to go offline";

/// The one sentence a chat is refused with off the server (MOD-25).
///
/// `htui` is online-only: since MOD-25 [`Backend::writer`](crate::Backend::writer) answers `None`
/// on [`Backend::Offline`](crate::Backend::Offline), so a chat started on a box whose Postgres is
/// unreachable is **refused** rather than recorded into `<cache_dir>/pending/`. This is the
/// sentence it is refused with, and it echoes `R-STO-4`'s "the TUI opens in offline read-only mode
/// from the cache ... No item creation, no runs": the shell still browses the mirror, with the top
/// bar reading `offline · <age>`, and starts no run.
///
/// It does **not** name the database itself, because it is carried by
/// [`StoreError::Unreachable`], whose `Display` already
/// prefixes `store unreachable: `. Reading the two together is the whole sentence; repeating the
/// word here made the rendered line say "unreachable" twice.
pub const DATABASE_UNREACHABLE: &str = "this box browses its read-only cache and starts no run";

/// [`DATABASE_UNREACHABLE`] for all 31 hierarchy methods (MOD-15 plan D2), reads included.
///
/// Reusing MOD-25's sentence rather than coining a thirty-second one is the decision, not an
/// economy. Writing: `htui` is online-only, so a workspace rename off the server is the same
/// event `R-STO-4` already words — "offline read-only mode … no item creation". Reading:
/// `workspace`, `repo`, `repo_box_path`, `workspace_box_path`, `step_graph` and
/// `step_graph_phase` have no mirror at all (`cache::MIRRORED_TABLES`), so offline they are
/// unreachable in the plainest sense of the word.
///
/// [`REGISTRY_ON_SERVER_ONLY`] and [`PROMPT_ON_SERVER_ONLY`] were the other candidates and both
/// name a different subsystem; a maintainer who met "the agent registry is written on the server
/// only" after renaming a project would go looking in the wrong place.
fn hierarchy_needs_the_server() -> StoreError {
    StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())
}

/// Plain delegation: a `Writer` decides *which* store, never *what* a read means.
impl ReadStore for Writer {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        match self {
            Self::Memory(store) => store.items(scope, filter).await,
            Self::Online(pg) => pg.items(scope, filter).await,
            Self::Buffered(buffer) => buffer.items(scope, filter).await,
        }
    }

    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        match self {
            Self::Memory(store) => store.item(id).await,
            Self::Online(pg) => pg.item(id).await,
            Self::Buffered(buffer) => buffer.item(id).await,
        }
    }

    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        match self {
            Self::Memory(store) => store.links(id, hops).await,
            Self::Online(pg) => pg.links(id, hops).await,
            Self::Buffered(buffer) => buffer.links(id, hops).await,
        }
    }

    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        match self {
            Self::Memory(store) => store.documents(id).await,
            Self::Online(pg) => pg.documents(id).await,
            Self::Buffered(buffer) => buffer.documents(id).await,
        }
    }

    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        match self {
            Self::Memory(store) => store.notes(id).await,
            Self::Online(pg) => pg.notes(id).await,
            Self::Buffered(buffer) => buffer.notes(id).await,
        }
    }

    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        match self {
            Self::Memory(store) => store.runs(id).await,
            Self::Online(pg) => pg.runs(id).await,
            Self::Buffered(buffer) => buffer.runs(id).await,
        }
    }

    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        match self {
            Self::Memory(store) => store.step_events(step).await,
            Self::Online(pg) => pg.step_events(step).await,
            Self::Buffered(buffer) => buffer.step_events(step).await,
        }
    }

    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        match self {
            Self::Memory(store) => store.document(id).await,
            Self::Online(pg) => pg.document(id).await,
            Self::Buffered(buffer) => buffer.document(id).await,
        }
    }

    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        match self {
            Self::Memory(store) => store.documents_of_kinds(item, kinds).await,
            Self::Online(pg) => pg.documents_of_kinds(item, kinds).await,
            Self::Buffered(buffer) => buffer.documents_of_kinds(item, kinds).await,
        }
    }

    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        match self {
            Self::Memory(store) => store.upstream_summaries(id, hops, scope).await,
            Self::Online(pg) => pg.upstream_summaries(id, hops, scope).await,
            Self::Buffered(buffer) => buffer.upstream_summaries(id, hops, scope).await,
        }
    }

    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        match self {
            Self::Memory(store) => store.project(id).await,
            Self::Online(pg) => pg.project(id).await,
            Self::Buffered(buffer) => buffer.project(id).await,
        }
    }
}

impl WriteStore for Writer {
    async fn mint_item(&self, new: NewItem) -> Result<Item> {
        match self {
            Self::Memory(store) => store.mint_item(new).await,
            Self::Online(pg) => pg.mint_item(new).await,
            Self::Buffered(buffer) => buffer.mint_item(new).await,
        }
    }

    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> Result<UpdateOutcome> {
        match self {
            Self::Memory(store) => store.update_item(id, expected_version, patch).await,
            Self::Online(pg) => pg.update_item(id, expected_version, patch).await,
            Self::Buffered(buffer) => buffer.update_item(id, expected_version, patch).await,
        }
    }

    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        match self {
            Self::Memory(store) => store.transition(id, from, to).await,
            Self::Online(pg) => pg.transition(id, from, to).await,
            Self::Buffered(buffer) => buffer.transition(id, from, to).await,
        }
    }

    async fn append_events(&self, events: &[SessionEvent]) -> Result<usize> {
        match self {
            Self::Memory(store) => store.append_events(events).await,
            Self::Online(pg) => pg.append_events(events).await,
            Self::Buffered(buffer) => buffer.append_events(events).await,
        }
    }

    async fn set_step_usage(
        &self,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.set_step_usage(step, usage, prompt_digest).await,
            Self::Online(pg) => pg.set_step_usage(step, usage, prompt_digest).await,
            Self::Buffered(buffer) => buffer.set_step_usage(step, usage, prompt_digest).await,
        }
    }

    async fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_agent(agent).await,
            Self::Online(pg) => pg.upsert_agent(agent).await,
            Self::Buffered(buffer) => buffer.upsert_agent(agent).await,
        }
    }

    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_agent_box(row).await,
            Self::Online(pg) => pg.upsert_agent_box(row).await,
            Self::Buffered(buffer) => buffer.upsert_agent_box(row).await,
        }
    }

    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => {
                store
                    .set_agent_box_quota(agent_id, box_id, quota, quota_at)
                    .await
            }
            Self::Online(pg) => {
                pg.set_agent_box_quota(agent_id, box_id, quota, quota_at)
                    .await
            }
            Self::Buffered(buffer) => {
                buffer
                    .set_agent_box_quota(agent_id, box_id, quota, quota_at)
                    .await
            }
        }
    }

    async fn start_chat_run(&self, chat: &ChatRunSpec) -> Result<()> {
        match self {
            Self::Memory(store) => store.start_chat_run(chat).await,
            Self::Online(pg) => pg.start_chat_run(chat).await,
            Self::Buffered(buffer) => buffer.start_chat_run(chat).await,
        }
    }

    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.finish_chat_run(run, step, status, finished_at).await,
            Self::Online(pg) => pg.finish_chat_run(run, step, status, finished_at).await,
            Self::Buffered(buffer) => buffer.finish_chat_run(run, step, status, finished_at).await,
        }
    }

    async fn set_step_prompt(&self, step: StepId, digest: &str, trim: &Value) -> Result<()> {
        match self {
            Self::Memory(store) => store.set_step_prompt(step, digest, trim).await,
            Self::Online(pg) => pg.set_step_prompt(step, digest, trim).await,
            Self::Buffered(buffer) => buffer.set_step_prompt(step, digest, trim).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Backend;
    use htui_core::fixtures::ids;

    #[tokio::test]
    async fn a_memory_writer_round_trips_a_chat_run() {
        let store = MemStore::demo();
        let backend = Backend::memory(store.clone());
        let writer = backend.writer().expect("a memory backend is writable");
        assert_eq!(writer.label(), "memory");

        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let before = store.active_runs(&scope).await.expect("count");

        let chat = ChatRunSpec::mint(
            ids::PROJECT_HTUI,
            ids::BOX,
            ids::USER,
            Some(ids::AGENT_CLAUDE),
            None,
        );
        writer.start_chat_run(&chat).await.expect("the rows mint");
        assert_eq!(
            store.active_runs(&scope).await.expect("count"),
            before + 1,
            "a live chat counts as an active run"
        );

        writer
            .finish_chat_run(
                chat.run_id,
                chat.step_id,
                RunStatus::Done,
                htui_core::fixtures::demo_at(0, 0),
            )
            .await
            .expect("the run closes");
        assert_eq!(
            store.active_runs(&scope).await.expect("count"),
            before,
            "a closed chat stops counting"
        );
    }

    #[tokio::test]
    async fn an_offline_backend_hands_out_no_writer_and_no_user() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "writer-test", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        assert!(
            backend.writer().is_none(),
            "since MOD-25 there is no offline write path at all: a chat off the server is \
             refused, not buffered"
        );
        assert!(
            !backend.is_writable(),
            "and the server is still unreachable: the re-dial ticker keys on this"
        );
        assert!(
            matches!(backend.this_user().await, Err(StoreError::NotFound { .. })),
            "and no author for a run row: this mirror has never synced one (MOD-2 D33)"
        );
        cache.close().await;
    }

    #[tokio::test]
    async fn a_memory_backend_answers_the_fixture_user() {
        let backend = Backend::memory(MemStore::demo());
        assert_eq!(
            backend.this_user().await.expect("the fixture has a user"),
            ids::USER
        );

        let empty = Backend::memory(MemStore::new());
        assert!(
            matches!(empty.this_user().await, Err(StoreError::NotFound { .. })),
            "an empty store has no user to start a run as"
        );
    }
}
