//! `impl WriteStore for PgStore` (blueprint C.9): the three write paths of ANA-9 §4.1 and §4.2,
//! plus MOD-2's six (plan D3, `docs/ANA-4.md` §4.1).
//!
//! Each one is a **single statement**, so atomicity comes from Postgres rather than from an
//! explicit transaction: a `WITH ... RETURNING` CTE either lands whole or not at all, and under
//! `READ COMMITTED` the loser of a compare-and-set race blocks on the row lock, re-evaluates
//! `version = $2` against the committed value and matches nothing (ANA-9 §11.3). The exceptions are
//! MOD-2's chat-run pair, which writes one `run` and one `run_step` and therefore takes an explicit
//! transaction, and MOD-15's two deletes, which count and delete in one transaction **at
//! `REPEATABLE READ`** so that the two statements share a snapshot as well (review M1; see
//! [`begin_repeatable_read`]). MOD-7's `record_box_probe` is one more: it updates the `box` row and
//! replaces its `box_tool` set, so it takes a transaction too (plan D10). MOD-38's amend, withdraw
//! and `cite` also take one, because each decides a refusal on a row it has locked before it writes
//! (see [`revise_requirement`]).
//!
//! There is **no `DELETE FROM item`** in this file and none may be added: §4.1's "keys are never
//! reused" is enforced by the absence of the path, and the conformance case `no_delete_path` is
//! what pins it. Nor does any statement write `updated_at` on an update path - the `BEFORE UPDATE`
//! trigger of the migration owns it, and `RETURNING` sees the trigger-modified row.

use std::collections::BTreeSet;

use chrono::{DateTime, TimeDelta, Utc};
use htui_core::model::link::{ItemLink, LinkKind, ProposeLink, WithdrawLink};
use htui_core::model::{
    Activation, Agent, AgentBox, AgentId, AnswerOutcome, BOX_PROBE_SPEC_KEY, BatchClose, BatchId,
    BindingChange, BoxEdit, BoxId, BoxProbe, BoxRecord, BoxRow, BoxSettings, BoxTool,
    CancelRequest, ChatRunSpec, CitationKind, Claim, CommandRun, CommandRunId, CommandRunStatus,
    DEFAULT_MAX_CONCURRENT_ITEMS, Document, Executor, FollowUpRequest, FollowUpSettle, GateOutcome,
    Isolation, Item, ItemId, ItemKind, ItemKindId, ItemKindPatch, ItemPatch, ItemRequirement,
    ItemRevision, NewCommandRun, NewDocument, NewFollowUp, NewItem, NewItemKind, NewNote,
    NewPersona, NewProject, NewPromptTemplate, NewRepo, NewRequirement, NewRequirementArea, NewRun,
    NewRunStep, NewSkill, NewSkillVersion, NewStepGraph, NewWorkspace, Note, OpenPermission,
    PermissionChoice, PermissionId, PermissionStatus, Persona, PersonaId, PersonaPatch,
    PersonaPermission, PersonaTools, PhaseAgent, PhaseId, PhasePatch, Priority, Project, ProjectId,
    ProjectPatch, PromptTemplate, PromptTemplateId, QueueBatch, QueueEntry, QueueMove,
    QueueSetting, QueuedFollowUp, RelaySessionId, RelayView, Repo, RepoBoxPath, RepoId, RepoPatch,
    Requirement, RequirementArea, RequirementAreaId, RequirementId, RequirementPatch,
    RequirementRevision, RequirementSpec, RequirementState, RequirementUpdate, Resolution, Run,
    RunCommand, RunCommandId, RunCommandStatus, RunId, RunKind, RunMode, RunStatus, RunStep,
    RunStepCommit, RunStepTree, Scope, SessionEvent, SettleOutcome, Skill, SkillBinding,
    SkillBindingId, SkillBindingKey, SkillId, SkillPatch, SkillVersion, Status, StepGraph,
    StepGraphId, StepGraphPatch, StepGraphPhase, StepId, StepOpening, StepOutcome, StepPermission,
    StepStatus, UserId, VerifyOutcome, WaitingPermission, Workspace, WorkspaceBoxPath, WorkspaceId,
    WorkspacePatch, WorkspaceProject, canonical_declared_tags, missing_tags_failure, moved_order,
    overlaps, scope_of,
};
use htui_core::prompt::settings::{SettingKey, rung_refusal, validate};
use htui_core::prompt::{DEFAULT_TEMPLATES, TemplateRole};
use htui_core::seed;
use htui_core::store::traits::{
    BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN, BOX_PROBE_SPEC_NOT_AN_OBJECT, BOX_SETTINGS_NOT_AN_OBJECT,
    COMMAND_STALE_AFTER, EXECUTOR_MUST_BE_KNOWN, batch_is_closed, command_finish_status,
    command_not_claimable, command_not_queued, document_needs_a_step, link_key,
    link_not_proposed_by_run, link_outside_project, note_needs_a_step,
    project_settings_not_an_object, reaped_note, self_link, step_document_refusal,
    step_note_refusal, step_writes_own_item,
};
use htui_core::store::{
    BindingFacts, CasOutcome, DeleteReach, DeleteTarget, ParkOutcome, QueueStored, QueueTarget,
    QueueToken, ReadStore as _, Result, SettingRung, StepFence, StoreError, StoredSetting,
    TransitionLaw, UpdateOutcome, WriteStore, already_exists, chat_step_status, check_attachment,
    citation_key, close_out_needs_a_summary, expected_on_row, failure_disagrees_with_status,
    finish_run_item_mirror, finish_run_needs_a_terminal_status, graph_not_in_project, illegal_move,
    invalid_area_code, invalid_prefix, item_has_a_live_run, item_kind_is_held, item_not_in_project,
    lease_ttl_micros, legal_move, new_persona_refusal, new_skill_refusal, not_a_fanout_candidate,
    not_a_terminal_status, persona_is_bound, persona_patch_refusal, prompt_template_key,
    prompt_template_refusal, queue_target_refusal, queue_token_refusal, references_no_row,
    requirement_withdrawn, reserved_phase_name, resolution_not_closable, row_names_another_phase,
    row_names_another_step, run_is_terminal, skill_body_refusal, skill_patch_refusal,
    skill_version_key, step_is_not_promotable, summary_names_another_item, winner_is_not_settled,
    withdrawn_requirement_cited,
};
use serde_json::Value;
use sqlx::PgConnection;
use sqlx::types::Json;
use uuid::Uuid;

use crate::error::map_sqlx;
use crate::pg::PgStore;
use crate::pg::rows::PersonaRow;

/// The refusal text for a kind that is unknown or belongs to another project.
///
/// ANA-9 §5.5 has no composite `(project_id, id)` key on `item_kind`, so this rule is an explicit
/// guard rather than a foreign key, on both write paths (blueprint H.13).
fn kind_not_in_project(kind: impl core::fmt::Display, project: impl core::fmt::Display) -> String {
    format!("item_kind `{kind}` does not exist in project `{project}`")
}

// ------------------------------------------------------------------------------------------------
// MOD-15 milestone 1 helpers (plan D3, D4, D8).
// ------------------------------------------------------------------------------------------------

/// The other half of a compare-and-set that touched no row (D3).
///
/// `rows_affected() == 0` means **either** the token the caller edited from is spent **or** there
/// is no such row, and those are different answers: the first hands the editor the row to reload
/// from, the second says there is nothing to reload. One follow-up read decides, which is the same
/// shape [`WriteStore::transition`]'s `SELECT 1 FROM item` has carried since MOD-1 — and the reason
/// `Stale` and `NotFound` are never confused on this backend.
fn cas_miss<T>(
    current: Option<T>,
    entity: &'static str,
    id: impl core::fmt::Display,
) -> Result<CasOutcome<T>> {
    current
        .map(CasOutcome::Stale)
        .ok_or_else(|| StoreError::NotFound {
            entity,
            id: id.to_string(),
        })
}

/// The refusal a pair outside ANA-2 §4.3 gets, in plan D14's order (MOD-4 T2).
///
/// The law is checked **before** the update, so an illegal pair never writes; but precedence says
/// the row is looked up first, and an id that names nothing is [`StoreError::NotFound`] even when
/// its `(from, to)` is also illegal. `exists` is that lookup's answer, which the caller runs
/// because `query_scalar!` needs a literal table name.
///
/// The cost is one extra round trip on a path that is already an error: every legal `(from, to)`
/// goes straight to its compare-and-set and never reaches here (blueprint §3.6).
///
/// `refusal` is [`legal_move`]'s own error, never a second copy of the rule: plan D15 gives the
/// §4.3 tables one home that both stores call, so a change there reaches `PgStore` without anyone
/// remembering to make it twice. All this function decides is D14's precedence — which of the two
/// answers the caller gets.
fn refuse_illegal_move<T: TransitionLaw, R>(
    exists: bool,
    id: impl core::fmt::Display,
    refusal: StoreError,
) -> Result<R> {
    if exists {
        Err(refusal)
    } else {
        Err(StoreError::NotFound {
            entity: T::ENTITY,
            id: id.to_string(),
        })
    }
}

/// Takes the project's `run_step_commit` and `run_step_tree` rows out of the way before its
/// `DELETE FROM project`, on the delete's own transaction (MOD-4 T2).
///
/// Both tables reference `repo(id)` **without** `ON DELETE CASCADE` (`0001_init.sql:503`,
/// `0003_orchestration.sql:94`) - deliberately: a tree outlives its repo's rename, never its step.
/// A project delete cascades to `repo` *and* to `run_step` in one statement, and Postgres checks
/// each foreign key as the row it guards is reached rather than at the end of the statement. So a
/// project holding a tree or a commit against one of its **own** repos raised a bare `23503` and
/// could not be deleted at all, where PRD D13 promises a report. `pg_criteria.rs`'s
/// `project_delete_takes_everything_and_says_so` named the hazard and routed around it while
/// nothing in the tree wrote either table; MOD-4's
/// [`upsert_step_tree`](WriteStore::upsert_step_tree) and
/// [`record_commits`](WriteStore::record_commits) are what made it reachable, and this is the fix.
///
/// The predicate is [`project_reach`]'s own, the rows of the project's steps and so the rows it
/// counted, which is why "the counts shown are the counts the act took" still holds: these rows
/// would have been cascaded away by `run_step` anyway, and are merely taken a statement earlier. A
/// row belonging to
/// *another* project's step but pointing at this project's repo is not touched and still refuses
/// the delete, which is the honest answer: nothing counted it.
async fn release_repo_references(conn: &mut PgConnection, id: ProjectId) -> Result<()> {
    sqlx::query!(
        r#"
        WITH s AS (SELECT id FROM run_step
                    WHERE run_id IN (SELECT id FROM run
                                      WHERE project_id = $1
                                         OR item_id IN (SELECT id FROM item WHERE project_id = $1))),
             c AS (DELETE FROM run_step_commit WHERE run_step_id IN (SELECT id FROM s))
        DELETE FROM run_step_tree WHERE run_step_id IN (SELECT id FROM s)
        "#,
        id.as_uuid(),
    )
    .execute(conn)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// The existence check the two batch upserts shared, on **their own transaction** (MOD-4 T2); since
/// MOD-41 plan D1 they run [`step_fence`] instead, and this serves `record_command_run`,
/// `interrupt_step`, `close_out` and [`fenced_or_missing`].
///
/// `run_step_tree` and `run_step_commit` both hang off `run_step` by a foreign key, so an unknown
/// step would already be a `23503` - but that is [`StoreError::Constraint`] where the contract says
/// [`StoreError::NotFound`], and an **empty** batch inserts nothing and so would raise nothing at
/// all. One `SELECT 1` inside the transaction answers both, and answers them the same way for a
/// batch of zero rows as for a batch of ten.
async fn step_exists(conn: &mut PgConnection, step: StepId) -> Result<()> {
    sqlx::query_scalar!("SELECT 1 FROM run_step WHERE id = $1", step.as_uuid())
        .fetch_optional(conn)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "run_step",
            id: step.to_string(),
        })?;
    Ok(())
}

/// MOD-77 plan D1: the lock [`step_fence`] takes on the step's row. The run's is always
/// `FOR SHARE`, and is always taken **after** the step's.
///
/// `FOR KEY SHARE` (review L1) is the lock an insert's foreign key takes on the step anyway. It
/// still conflicts with a park's `FOR UPDATE OF s`, so the writer queues at the step before it
/// touches the run (the step → run order), and it does not conflict with `FOR NO KEY UPDATE`, so a
/// reader-inserter does not serialise behind the step's own usage, settle and transition writes.
/// [`step_scope`] keeps MOD-11's `FOR SHARE OF s, r`: it is out of MOD-77's scope, and the
/// stronger step lock changes no lock order, only how much it waits.
#[derive(Clone, Copy, Debug)]
enum StepLock {
    /// `FOR KEY SHARE OF s FOR SHARE OF r`: the caller reads the step and inserts rows whose
    /// foreign key names it, and never updates it (`record_commits`, [`fenced_miss`]).
    KeyShare,
    /// `FOR NO KEY UPDATE OF s FOR SHARE OF r`: the caller goes on to `UPDATE run_step`
    /// (`upsert_step_tree`'s `isolation_path`), so it takes the update lock up front. A share
    /// lock upgraded later would deadlock two such writers of one step against each other.
    Update,
}

/// MOD-41 plan D1: `step` exists and its run carries `fence`'s lease, read under row locks on the
/// step and then the run, inside the caller's transaction, so an adoption cannot commit between
/// this check and the batch's writes. [`StoreError::NotFound`] first, then
/// [`StoreError::Fenced`]: `append_events`' order. The fenced twin of [`step_exists`], for
/// `upsert_step_tree`, `record_commits` and [`fenced_miss`]; `close_out` keeps the unfenced
/// check (blueprint B-1).
///
/// MOD-77 plan D1: the step is locked **before** the run, `park_step`'s `FOR UPDATE OF s, r`
/// order. MOD-41 locked the run alone (`FOR SHARE OF r`) and left the step to the writes that
/// follow: an insert's foreign key takes `FOR KEY SHARE` on it, and `upsert_step_tree`'s
/// `UPDATE run_step` a row lock. Against a park that held the step and waited for the run, that
/// closed a cycle and Postgres aborted one side (`40P01`). Holding the step first makes
/// whichever of the two reaches it first run to its commit. The clause order is load-bearing:
/// Postgres locks a joined row in the order the locking clauses are written. `lock` chooses the
/// step's mode ([`StepLock`]); `pg_criteria.rs`'s `upsert_step_tree_takes_the_step_first` and
/// `record_commits_takes_the_step_first` pin the order.
async fn step_fence(
    conn: &mut PgConnection,
    step: StepId,
    fence: StepFence,
    lock: StepLock,
) -> Result<()> {
    // Two statements, not one with a spliced clause: `query_scalar!` needs literal SQL.
    let owner = match lock {
        StepLock::KeyShare => {
            sqlx::query_scalar!(
                r#"SELECT r.lease_owner AS "lease_owner?"
                     FROM run_step s JOIN run r ON r.id = s.run_id
                    WHERE s.id = $1
                      FOR KEY SHARE OF s FOR SHARE OF r"#,
                step.as_uuid(),
            )
            .fetch_optional(conn)
            .await
        }
        StepLock::Update => {
            sqlx::query_scalar!(
                r#"SELECT r.lease_owner AS "lease_owner?"
                     FROM run_step s JOIN run r ON r.id = s.run_id
                    WHERE s.id = $1
                      FOR NO KEY UPDATE OF s FOR SHARE OF r"#,
                step.as_uuid(),
            )
            .fetch_optional(conn)
            .await
        }
    }
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::NotFound {
        entity: "run_step",
        id: step.to_string(),
    })?;
    if owner == fence.owner() {
        Ok(())
    } else {
        Err(StoreError::Fenced { step })
    }
}

/// MOD-11 F-21: what [`step_scope`] answers — the step's run, its item (`None` for a chat run)
/// and its project.
struct StepScope {
    /// `run.id`.
    run: RunId,
    /// `run.item_id`.
    item: Option<ItemId>,
    /// `run.project_id`.
    project: ProjectId,
}

/// MOD-11 F-21: [`step_fence`] that also answers the run's id, item and project, read in the
/// caller's transaction. [`StoreError::NotFound`] first, then [`StoreError::Fenced`]:
/// `step_fence`'s order.
///
/// It locks the **step** as well as the run (`FOR SHARE OF s, r`, the order of `park_step`'s
/// `FOR UPDATE OF s, r`, which [`step_fence`] has taken too since MOD-77). The writes that follow
/// insert a row whose foreign key takes `FOR KEY SHARE` on the step; with the run alone held, a
/// park that had locked the step and was waiting for the run made that key lock wait on the park,
/// and the two deadlocked (`40P01`, round 0 of
/// `pg_criteria.rs::a_step_document_racing_a_park_never_deadlocks`). Holding the step first
/// makes whichever of the two reaches it first run to its commit.
async fn step_scope(conn: &mut PgConnection, step: StepId, fence: StepFence) -> Result<StepScope> {
    let row = sqlx::query!(
        r#"SELECT r.id          AS "run_id: RunId",
                  r.item_id     AS "item_id: ItemId",
                  r.project_id  AS "project_id: ProjectId",
                  r.lease_owner AS "lease_owner?"
             FROM run_step s JOIN run r ON r.id = s.run_id
            WHERE s.id = $1
              FOR SHARE OF s, r"#,
        step.as_uuid(),
    )
    .fetch_optional(conn)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::NotFound {
        entity: "run_step",
        id: step.to_string(),
    })?;
    if row.lease_owner != fence.owner() {
        return Err(StoreError::Fenced { step });
    }
    Ok(StepScope {
        run: row.run_id,
        item: row.item_id,
        project: row.project_id,
    })
}

/// MOD-40 plan D1: why a fenced `UPDATE` of `step` matched no row. [`StoreError::NotFound`] when
/// the step does not exist, else [`StoreError::Fenced`]: `step_exists`' follow-up read on a miss,
/// `interrupt_step`'s shape.
///
/// Its callers box it: it is the rare branch, and inline it would add an acquire and a query to
/// every settle's future, which the engine nests deep enough that a debug build's worker stack
/// overflows (`htui/tests/runs_pg.rs`).
async fn fenced_or_missing(pool: &sqlx::PgPool, step: StepId) -> StoreError {
    let exists = async {
        let mut conn = pool.acquire().await.map_err(map_sqlx)?;
        step_exists(&mut conn, step).await
    };
    match exists.await {
        Ok(()) => StoreError::Fenced { step },
        Err(err) => err,
    }
}

/// MOD-37 R-5: why a fenced compare-and-set of `step` matched no row. [`step_fence`] answers
/// [`StoreError::NotFound`] and then [`StoreError::Fenced`]; a step that passes both is simply not
/// at the status the write wanted, `Ok(false)`. Boxed by its callers for [`fenced_or_missing`]'s
/// stack reason.
///
/// [`StepLock::KeyShare`] (MOD-77 plan D1, review L1): it runs on a pooled connection outside any
/// transaction, so the lock lasts one statement and only makes this read wait for a park holding
/// the step or a lease writer holding its run; no `UPDATE` follows that would need the step's
/// update lock.
async fn fenced_miss(pool: &sqlx::PgPool, step: StepId, fence: StepFence) -> Result<bool> {
    let mut conn = pool.acquire().await.map_err(map_sqlx)?;
    step_fence(&mut conn, step, fence, StepLock::KeyShare).await?;
    Ok(false)
}

/// The `document` insert [`WriteStore::write_document`] and [`WriteStore::close_out`] share, on
/// **their own transaction** and under a lock the caller has already taken on the item (MOD-4 T2).
///
/// The version is allocated by the statement itself - `COALESCE(MAX(version), 0) + 1` over the
/// `(item, kind)` the row is about to join - so the read and the write are one snapshot. What makes
/// that safe against a second writer is the caller's `SELECT ... FOR UPDATE` on the **item**: the
/// row the second writer would have to wait for does not exist in `document` yet, so the parent is
/// the only row both are guaranteed to contend on.
///
/// `close_out` cannot call [`WriteStore::write_document`] for this, because that method opens a
/// transaction of its own and close-out's three effects are one (the same reason `seed_project`
/// does not reuse `create_step_graph`).
///
/// # Errors
///
/// [`StoreError::Constraint`] on a duplicate id (`23505`) or a `created_by` /
/// `produced_by_step_id` that names no row (`23503`); the item's own key cannot fail here, because
/// the caller has just read the row.
async fn insert_document(conn: &mut PgConnection, new: NewDocument) -> Result<Document> {
    sqlx::query_as!(
        Document,
        r#"
        INSERT INTO document (id, item_id, kind, version, title, body, produced_by_step_id,
                              created_by, created_at)
        SELECT $1, $2, $3, COALESCE(MAX(version), 0) + 1, $4, $5, $6, $7, $8
          FROM document WHERE item_id = $2 AND kind = $3
        RETURNING id                  AS "id: htui_core::model::DocumentId",
                  item_id             AS "item_id: ItemId",
                  kind,
                  version,
                  title,
                  body,
                  produced_by_step_id AS "produced_by_step_id: StepId",
                  created_by          AS "created_by: UserId",
                  created_at
        "#,
        new.id.as_uuid(),
        new.item_id.as_uuid(),
        new.kind,
        new.title,
        new.body,
        new.produced_by_step_id.map(StepId::as_uuid),
        new.created_by.as_uuid(),
        new.created_at,
    )
    .fetch_one(conn)
    .await
    .map_err(map_sqlx)
}

/// A `count(*)` as the `u64` [`DeleteReach`] holds. Postgres counts as `bigint` and never
/// negative, so the `unwrap_or` is unreachable rather than a policy.
fn rows(count: i64) -> u64 {
    u64::try_from(count).unwrap_or(0)
}

/// How many times the two delete paths re-run their whole transaction after a `40001` before they
/// give up and refuse (review M1).
///
/// Three rather than one because a retry is cheap — the transaction holds no lock it has not just
/// taken — and rather than unbounded because a project under a steady stream of writes would
/// otherwise never return.
const DELETE_ATTEMPTS: u32 = 3;

/// How many guard-then-holders passes `delete_persona` makes before it gives up (MOD-26 M2 R1
/// L-3, Deviation D-11): the holders only shrink under the persona's `FOR UPDATE`, so a real race
/// ends in a pass or two, and a third that still finds the guard fired over no nameable holder
/// is answered as a backend error rather than looped on.
const DELETE_PERSONA_PASSES: u32 = 3;

/// Opens a transaction at `REPEATABLE READ`, which is where the two delete paths count (review M1).
///
/// `READ COMMITTED` gives every statement its own snapshot, so a `count(*)` and a `DELETE` in one
/// transaction are still two views of the table: a child row committed between them is cascaded
/// away uncounted. At `REPEATABLE READ` the whole transaction reads one snapshot **and** the
/// cascade's own referential-integrity queries run with a crosscheck against it, so such a row
/// raises `40001` instead of vanishing silently — which is what makes PRD D13's "the counts shown
/// match what the cascade removes" a property of the code rather than of the timing.
///
/// `SET TRANSACTION` must be the transaction's first statement, which is why this is a helper
/// rather than a line in each caller.
async fn begin_repeatable_read(
    pool: &sqlx::PgPool,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    Ok(tx)
}

/// Whether a driver error is `40001`, *serialization failure*: the one outcome [`DELETE_ATTEMPTS`]
/// retries.
///
/// Not folded into [`map_sqlx`], which would turn a retryable answer into the same
/// [`StoreError::Backend`] every other class-`40` failure gets: only the two delete paths open a
/// transaction that can raise it, and only they know the work is safe to repeat.
fn is_serialization_failure(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().is_some_and(|code| code == "40001"))
}

/// What a workspace delete reaches: its links and its box paths, and no project
/// (`0001_init.sql:162,174`). `None` when no row has that id.
///
/// Takes a connection rather than the pool so [`WriteStore::delete_reach`] can count on a borrowed
/// one and [`WriteStore::delete_workspace`] can count **inside its own transaction** — which,
/// together with [`begin_repeatable_read`], is what makes "the counts shown before the act are the
/// counts the act took" (PRD D13) true of one snapshot rather than of two.
async fn workspace_reach(conn: &mut PgConnection, id: WorkspaceId) -> Result<Option<DeleteReach>> {
    let counted = sqlx::query!(
        r#"
        SELECT (SELECT count(*) FROM workspace_project  WHERE workspace_id = $1) AS "links!",
               (SELECT count(*) FROM workspace_box_path WHERE workspace_id = $1) AS "paths!"
          FROM workspace WHERE id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(conn)
    .await
    .map_err(map_sqlx)?;

    Ok(counted.map(|row| DeleteReach {
        workspace_links: rows(row.links),
        workspace_box_paths: rows(row.paths),
        ..DeleteReach::default()
    }))
}

/// What a project delete reaches, in one statement of scalar subqueries over `0001_init.sql`'s
/// `ON DELETE CASCADE` chain (PRD D13, plan V11). `None` when no row has that id.
///
/// The nine CTEs are the id sets the cascade walks, named as
/// [`MemStore`](htui_core::store::MemStore)'s `ProjectReach` names them, so the two backends count
/// the same rows: `run` is reached by `project_id` **or** by an `item_id` of the project
/// (`0001_init.sql:450` cascades from the item), and `item_link` by **either** end, tombstones
/// included — which is what takes the fixture's cross-project edge.
///
/// `command_run` hangs off `s` like `session_event` and `run_step_commit` do (`0001_init.sql:539`);
/// plan V11's list of the cascade left it out and this is where that is corrected (review L1).
///
/// MOD-38's `ra` and `rq` are the project's requirement areas and requirements
/// (`0006_requirements.sql`, blueprint §4.4). `item_requirement` is reached by **either** end, as
/// `item_link` is: a citation from another project's item goes with the requirement it cites.
///
/// `workspace_box_paths` is `0`: a project is not a workspace.
async fn project_reach(conn: &mut PgConnection, id: ProjectId) -> Result<Option<DeleteReach>> {
    let counted = sqlx::query!(
        r#"
        WITH i  AS (SELECT id FROM item             WHERE project_id = $1),
             r  AS (SELECT id FROM run              WHERE project_id = $1
                                                       OR item_id IN (SELECT id FROM i)),
             s  AS (SELECT id FROM run_step         WHERE run_id   IN (SELECT id FROM r)),
             g  AS (SELECT id FROM step_graph       WHERE project_id = $1),
             ph AS (SELECT id FROM step_graph_phase WHERE graph_id IN (SELECT id FROM g)),
             rp AS (SELECT id FROM repo             WHERE project_id = $1),
             t  AS (SELECT run_step_id FROM run_step_tree
                     WHERE run_step_id IN (SELECT id FROM s)),
             ra AS (SELECT id FROM requirement_area WHERE project_id = $1),
             rq AS (SELECT id FROM requirement      WHERE project_id = $1)
        SELECT (SELECT count(*) FROM workspace_project WHERE project_id = $1) AS "workspace_links!",
               (SELECT count(*) FROM i)                                       AS "items!",
               (SELECT count(*) FROM item_key_counter WHERE project_id = $1)  AS "item_key_counters!",
               (SELECT count(*) FROM item_kind        WHERE project_id = $1)  AS "item_kinds!",
               (SELECT count(*) FROM g)                                       AS "step_graphs!",
               (SELECT count(*) FROM ph)                                      AS "phases!",
               (SELECT count(*) FROM phase_agent
                 WHERE phase_id IN (SELECT id FROM ph))                       AS "phase_agents!",
               (SELECT count(*) FROM prompt_template  WHERE project_id = $1)  AS "prompt_templates!",
               (SELECT count(*) FROM rp)                                      AS "repos!",
               (SELECT count(*) FROM repo_box_path
                 WHERE repo_id IN (SELECT id FROM rp))                        AS "repo_box_paths!",
               (SELECT count(*) FROM skill_binding    WHERE project_id = $1)  AS "skill_bindings!",
               (SELECT count(*) FROM r)                                       AS "runs!",
               (SELECT count(*) FROM s)                                       AS "run_steps!",
               (SELECT count(*) FROM session_event
                 WHERE run_step_id IN (SELECT id FROM s))                     AS "session_events!",
               (SELECT count(*) FROM run_step_commit
                 WHERE run_step_id IN (SELECT id FROM s))                     AS "run_step_commits!",
               (SELECT count(*) FROM t)                                       AS "run_step_trees!",
               (SELECT count(*) FROM command_run
                 WHERE run_step_id IN (SELECT id FROM s))                     AS "command_runs!",
               (SELECT count(*) FROM item_note
                 WHERE item_id IN (SELECT id FROM i))                         AS "notes!",
               (SELECT count(*) FROM item_revision
                 WHERE item_id IN (SELECT id FROM i))                         AS "revisions!",
               (SELECT count(*) FROM item_link
                 WHERE from_item_id IN (SELECT id FROM i)
                    OR to_item_id   IN (SELECT id FROM i))                    AS "links!",
               (SELECT count(*) FROM document
                 WHERE item_id IN (SELECT id FROM i))                         AS "documents!",
               (SELECT count(*) FROM requirement_spec WHERE project_id = $1)  AS "requirement_specs!",
               (SELECT count(*) FROM ra)                                      AS "requirement_areas!",
               (SELECT count(*) FROM requirement_key_counter
                 WHERE area_id IN (SELECT id FROM ra))                        AS "requirement_key_counters!",
               (SELECT count(*) FROM rq)                                      AS "requirements!",
               (SELECT count(*) FROM requirement_revision
                 WHERE requirement_id IN (SELECT id FROM rq))                 AS "requirement_revisions!",
               (SELECT count(*) FROM item_requirement
                 WHERE item_id IN (SELECT id FROM i)
                    OR requirement_id IN (SELECT id FROM rq))                 AS "item_requirements!"
          FROM project WHERE id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(conn)
    .await
    .map_err(map_sqlx)?;

    Ok(counted.map(|row| DeleteReach {
        workspace_links: rows(row.workspace_links),
        workspace_box_paths: 0,
        items: rows(row.items),
        item_key_counters: rows(row.item_key_counters),
        item_kinds: rows(row.item_kinds),
        step_graphs: rows(row.step_graphs),
        phases: rows(row.phases),
        phase_agents: rows(row.phase_agents),
        prompt_templates: rows(row.prompt_templates),
        repos: rows(row.repos),
        repo_box_paths: rows(row.repo_box_paths),
        skill_bindings: rows(row.skill_bindings),
        runs: rows(row.runs),
        run_steps: rows(row.run_steps),
        session_events: rows(row.session_events),
        run_step_commits: rows(row.run_step_commits),
        run_step_trees: rows(row.run_step_trees),
        command_runs: rows(row.command_runs),
        notes: rows(row.notes),
        revisions: rows(row.revisions),
        links: rows(row.links),
        documents: rows(row.documents),
        requirement_specs: rows(row.requirement_specs),
        requirement_areas: rows(row.requirement_areas),
        requirement_key_counters: rows(row.requirement_key_counters),
        requirements: rows(row.requirements),
        requirement_revisions: rows(row.requirement_revisions),
        item_requirements: rows(row.item_requirements),
    }))
}

// ------------------------------------------------------------------------------------------------
// MOD-38 helpers (ANA-11 §5.1, plan D9, D10).
// ------------------------------------------------------------------------------------------------

/// What an amend or a withdraw writes: the columns it moves (`None` leaves one as it is), the
/// revision it records and the deciding item's citation (PRD D3).
///
/// [`WriteStore::amend_requirement`] and [`WriteStore::withdraw_requirement`] differ only in these
/// values, so both are [`revise_requirement`] with a different edit.
struct RequirementEdit {
    body: Option<String>,
    rationale: Option<String>,
    priority: Option<Priority>,
    state: Option<RequirementState>,
    author_id: UserId,
    box_id: Option<BoxId>,
    reason: String,
    decided_by: ItemId,
    kind: CitationKind,
}

/// The compare-and-set of an amend or a withdraw, one transaction (plan D9, PRD D3).
///
/// The row is locked `FOR UPDATE` first, so the three refusals are decided against the version
/// the write will move, in the seam's order: NotFound, divergence, then a withdrawn requirement
/// ([`requirement_withdrawn`]). Under that lock one statement moves the row to `version + 1`,
/// inserts its revision and upserts the deciding item's citation at the new version, reviving a
/// tombstone. An `amended_by` / author / box that names no row fails that statement with `23503`,
/// and the transaction is dropped unwritten.
///
/// A divergence answers the revision at `expected_version` as its ancestor; a token newer than
/// the head has no such revision and is `NotFound { entity: "requirement_revision" }`, as
/// `update_item`'s is for `item_revision`.
async fn revise_requirement(
    pool: &sqlx::PgPool,
    id: RequirementId,
    expected_version: i32,
    edit: RequirementEdit,
) -> Result<RequirementUpdate> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;

    let Some(head) = sqlx::query_as!(
        Requirement,
        r#"
        SELECT id         AS "id: RequirementId",
               project_id AS "project_id: ProjectId",
               area_id    AS "area_id: RequirementAreaId",
               area_code,
               number,
               key        AS "key!",
               body,
               rationale,
               priority   AS "priority: Priority",
               state      AS "state: RequirementState",
               version,
               created_by AS "created_by: UserId",
               created_at,
               updated_at
          FROM requirement WHERE id = $1 FOR UPDATE
        "#,
        id.as_uuid(),
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?
    else {
        return Err(StoreError::NotFound {
            entity: "requirement",
            id: id.to_string(),
        });
    };

    if head.version != expected_version {
        let ancestor = sqlx::query_as!(
            RequirementRevision,
            r#"
            SELECT requirement_id     AS "requirement_id: RequirementId",
                   version,
                   body,
                   rationale,
                   priority           AS "priority: Priority",
                   state              AS "state: RequirementState",
                   author_id          AS "author_id: UserId",
                   box_id             AS "box_id: BoxId",
                   reason,
                   amended_by_item_id AS "amended_by_item_id: ItemId",
                   created_at
              FROM requirement_revision WHERE requirement_id = $1 AND version = $2
            "#,
            id.as_uuid(),
            expected_version,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "requirement_revision",
            id: format!("{id}@{expected_version}"),
        })?;
        return Ok(RequirementUpdate::Diverged { head, ancestor });
    }
    if head.state == RequirementState::Withdrawn {
        return Err(StoreError::Constraint(requirement_withdrawn(&head.key)));
    }

    let updated = sqlx::query_as!(
        Requirement,
        r#"
        WITH u AS (
            UPDATE requirement SET
                body      = COALESCE($2, body),
                rationale = COALESCE($3, rationale),
                priority  = COALESCE($4, priority),
                state     = COALESCE($5, state),
                version   = version + 1
             WHERE id = $1
            RETURNING id, project_id, area_id, area_code, number, key, body, rationale, priority,
                      state, version, created_by, created_at, updated_at
        ), v AS (
            INSERT INTO requirement_revision (requirement_id, version, body, rationale, priority,
                                              state, author_id, box_id, reason,
                                              amended_by_item_id)
            SELECT id, version, body, rationale, priority, state, $6, $7, $8, $9 FROM u
            RETURNING requirement_id
        ), c AS (
            INSERT INTO item_requirement (item_id, requirement_id, kind, requirement_version)
            SELECT $9, id, $10, version FROM u
            ON CONFLICT (item_id, requirement_id, kind)
            DO UPDATE SET requirement_version = EXCLUDED.requirement_version, deleted_at = NULL
            RETURNING item_id
        )
        SELECT u.id         AS "id!: RequirementId",
               u.project_id AS "project_id!: ProjectId",
               u.area_id    AS "area_id!: RequirementAreaId",
               u.area_code  AS "area_code!",
               u.number     AS "number!",
               u.key        AS "key!",
               u.body       AS "body!",
               u.rationale  AS "rationale!",
               u.priority   AS "priority!: Priority",
               u.state      AS "state!: RequirementState",
               u.version    AS "version!",
               u.created_by AS "created_by!: UserId",
               u.created_at AS "created_at!",
               u.updated_at AS "updated_at!"
          FROM u, v, c
        "#,
        id.as_uuid(),
        edit.body,
        edit.rationale,
        edit.priority.map(Priority::as_str),
        edit.state.map(RequirementState::as_str),
        edit.author_id.as_uuid(),
        edit.box_id.map(BoxId::as_uuid),
        edit.reason,
        edit.decided_by.as_uuid(),
        edit.kind.as_str(),
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    let Some(updated) = updated else {
        // Impossible for `close_out`'s reason: the row has been locked `FOR UPDATE` since this
        // transaction's first statement, and the `UPDATE` has no other conjunct.
        return Err(StoreError::Backend(format!(
            "requirement `{id}` did not move from version {expected_version} under the row lock \
             this transaction holds"
        )));
    };

    tx.commit().await.map_err(map_sqlx)?;
    Ok(RequirementUpdate::Updated(updated))
}

// ------------------------------------------------------------------------------------------------
// MOD-9 milestone 3 helpers (plan D75-D79, blueprint D89, D92).
// ------------------------------------------------------------------------------------------------

/// MOD-26 D4: `uq_persona_name`'s `23505` in `MemStore`'s sentence; anything else through
/// [`map_sqlx`].
fn persona_name_taken(err: sqlx::Error, name: &str) -> StoreError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("uq_persona_name") => {
            StoreError::Constraint(already_exists("persona", name))
        }
        _ => map_sqlx(err),
    }
}

/// `create_persona`'s refusals in `MemStore`'s order: `persona_pkey` is the id, then the name.
fn persona_insert_refused(err: sqlx::Error, new: &NewPersona) -> StoreError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("persona_pkey") => {
            StoreError::Constraint(already_exists("persona", new.id))
        }
        _ => persona_name_taken(err, &new.name),
    }
}

/// MOD-26 D5: `fk_step_graph_phase_persona`'s `23503` in `MemStore`'s sentence (every other
/// `23xxx` stays raw, `error.rs`); anything else through [`map_sqlx`].
fn phase_persona_refused(err: sqlx::Error, persona: Option<PersonaId>) -> StoreError {
    match (&err, persona) {
        (sqlx::Error::Database(db), Some(persona))
            if db.constraint() == Some("fk_step_graph_phase_persona") =>
        {
            StoreError::Constraint(references_no_row(
                "step_graph_phase.persona_id",
                persona,
                "persona",
            ))
        }
        _ => map_sqlx(err),
    }
}

/// A persona blob as `JSONB`, `create_run`'s shape.
fn persona_json<T: serde::Serialize>(column: &str, value: &T) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|error| StoreError::Constraint(format!("{column} does not serialise: {error}")))
}

/// D92 (F-P): `skill_name_key`'s `23505` in the sentence `MemStore` gives a taken name; anything
/// else through [`map_sqlx`].
fn skill_name_taken(err: sqlx::Error, name: &str) -> StoreError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("skill_name_key") => {
            StoreError::Constraint(already_exists("skill", name))
        }
        _ => map_sqlx(err),
    }
}

/// `create_skill`'s `INSERT INTO skill` refusals in `MemStore`'s sentences: `skill_pkey`'s `23505`
/// is [`already_exists`] on the id, then [`skill_name_taken`].
fn skill_insert_refused(err: sqlx::Error, new: &NewSkill) -> StoreError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("skill_pkey") => {
            StoreError::Constraint(already_exists("skill", new.id))
        }
        _ => skill_name_taken(err, &new.name),
    }
}

/// `MemStore`'s refusal of a `created_by` that names no `app_user`, the nil UUID worded apart.
fn unknown_author(column: &str, id: UserId) -> String {
    if id.as_uuid().is_nil() {
        format!("{column} must reference an app_user; the nil UUID does not")
    } else {
        format!("{column} `{id}` references no app_user")
    }
}

/// D89's classification of an append that wrote nothing, or of bad input before any write
/// (`refusal`), from two reads: a head other than `expected` is `Stale(head)`; then an unknown
/// skill is `NotFound`; then no version at all under a token other than `0` is `NotFound` on the
/// version; then the refusal, if any, is `Constraint`.
///
/// With none of those, the append lost to nothing: a concurrent append leaves a head above
/// `expected`, and `skill_version` rows are only ever removed with their skill, so this arm is
/// answered as a [`StoreError::Backend`] naming what was seen rather than guessed at.
async fn skill_version_miss(
    store: &PgStore,
    skill: SkillId,
    expected: i32,
    refusal: Option<String>,
) -> Result<CasOutcome<SkillVersion>> {
    let head = store.skill_version_rows(skill).await?.pop();
    if let Some(head) = head.as_ref()
        && head.version != expected
    {
        return Ok(CasOutcome::Stale(head.clone()));
    }
    if store.skill_row(skill).await?.is_none() {
        return Err(StoreError::NotFound {
            entity: "skill",
            id: skill.to_string(),
        });
    }
    if head.is_none() && expected != 0 {
        return Err(StoreError::NotFound {
            entity: "skill_version",
            id: skill_version_key(skill, expected),
        });
    }
    match refusal {
        Some(refusal) => Err(StoreError::Constraint(refusal)),
        None => Err(StoreError::Backend(format!(
            "skill {skill}: the append at head {expected} wrote nothing, and the head is still \
             {expected}"
        ))),
    }
}

/// [`WriteStore::finish_run`]'s whole body on `conn`, inside the caller's transaction: the run's
/// terminal move and the item's mirror ([`finish_run_item_mirror`]), with the run then the item
/// taken `FOR UPDATE`. `finish_run` wraps it in a transaction of its own; MOD-12's
/// [`close_batch`](PgStore::close_batch) runs it for each run of the batch still `queued`, in the
/// close's transaction (review H2).
async fn finish_run_on(
    conn: &mut PgConnection,
    run: RunId,
    to: RunStatus,
    failure: Option<&str>,
    at: DateTime<Utc>,
) -> Result<()> {
    // The raw SQL keeps the indentation it had inside `finish_run`, so its `.sqlx` entry stands.
    let Some(row) = sqlx::query!(
        r#"
            SELECT status  AS "status: RunStatus",
                   item_id AS "item_id: ItemId"
              FROM run
             WHERE id = $1
               FOR UPDATE
            "#,
        run.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_sqlx)?
    else {
        return Err(StoreError::NotFound {
            entity: "run",
            id: run.to_string(),
        });
    };

    if !to.is_terminal() {
        return Err(StoreError::Constraint(finish_run_needs_a_terminal_status(
            run, to,
        )));
    }
    if failure.is_some() != (to == RunStatus::Failed) {
        return Err(StoreError::Constraint(failure_disagrees_with_status(
            run,
            to,
            failure.is_some(),
        )));
    }
    // A terminal row reaches nothing, so `legal_move` is the whole refusal: `run_is_terminal`
    // would only say the same thing in a second sentence.
    legal_move(row.status, to)?;

    sqlx::query!(
        "UPDATE run \
            SET status      = $2, \
                failure     = COALESCE($3, failure), \
                finished_at = COALESCE(finished_at, $4) \
          WHERE id = $1",
        run.as_uuid(),
        to as RunStatus,
        failure,
        at,
    )
    .execute(&mut *conn)
    .await
    .map_err(map_sqlx)?;

    if let Some(item) = row.item_id {
        let Some(current) = sqlx::query_scalar!(
            r#"SELECT status AS "status: Status" FROM item WHERE id = $1 FOR UPDATE"#,
            item.as_uuid(),
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(map_sqlx)?
        else {
            // The foreign key makes this unreachable; it is an early return rather than a
            // panic because a missing parent is the database's fault, not the caller's.
            return Ok(());
        };

        let live = sqlx::query_scalar!(
            "SELECT count(*) FROM run \
              WHERE item_id = $1 AND id <> $2 \
                AND status IN ('queued','running','awaiting_approval')",
            item.as_uuid(),
            run.as_uuid(),
        )
        .fetch_one(&mut *conn)
        .await
        .map_err(map_sqlx)?
        .unwrap_or_default();

        if live == 0
            && let Some(target) = finish_run_item_mirror(to, current)
        {
            sqlx::query!(
                "UPDATE item \
                    SET status    = $3, \
                        closed_at = CASE WHEN $4 THEN clock_timestamp() ELSE NULL END \
                  WHERE id = $1 AND status = $2",
                item.as_uuid(),
                current.as_str(),
                target.as_str(),
                target.is_terminal(),
            )
            .execute(&mut *conn)
            .await
            .map_err(map_sqlx)?;
        }
    }

    Ok(())
}

impl WriteStore for PgStore {
    /// Mints an item: counter upsert, key assembly and revision 1 in one statement (ANA-9 §7.1).
    ///
    /// The kind-belongs-to-project guard is folded into the counter CTE's `SELECT`, so an invalid
    /// kind makes `c` empty and nothing is inserted anywhere - no key number is burned by a mint
    /// that never lands (`mint_unknown_kind_rejected`, `nil_author_rejected`). Zero rows out of the
    /// outer `SELECT` is therefore exactly "the guard fired".
    ///
    /// `key_prefix` comes from the kind, never from the caller ([`NewItem`] has no such field), and
    /// `r` is joined into the final `SELECT` so the dependency on the revision insert is written
    /// down rather than implied.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for an unknown or cross-project kind, for a duplicate `item.id`
    /// (`23505`) and for a `created_by` that names no `app_user` row (`23503`, which is what the
    /// nil UUID does).
    async fn mint_item(&self, new: NewItem) -> Result<Item> {
        let minted = sqlx::query_as!(
            Item,
            r#"
            WITH c AS (
                INSERT INTO item_key_counter (project_id, prefix, last_value)
                SELECT $2, k.prefix, 1 FROM item_kind k WHERE k.id = $3 AND k.project_id = $2
                ON CONFLICT (project_id, prefix)
                DO UPDATE SET last_value = item_key_counter.last_value + 1
                RETURNING prefix, last_value
            ), i AS (
                INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, body,
                                  priority, required_tags, touched_paths, step_graph_id, created_by)
                SELECT $1, $2, $3, c.prefix, c.last_value, $4, $5, $6, $7, $8, $9, $10 FROM c
                RETURNING id, project_id, kind_id, key_prefix, key_number, key, title, body, status,
                          priority, required_tags, touched_paths, step_graph_id, version,
                          created_by, created_at, updated_at, closed_at, resolution
            ), r AS (
                INSERT INTO item_revision (item_id, version, title, body, required_tags,
                                           author_id, box_id, reason)
                SELECT id, version, title, body, required_tags, $10, $11, 'created' FROM i
                RETURNING item_id
            )
            SELECT i.id            AS "id!: ItemId",
                   i.project_id    AS "project_id!: htui_core::model::ProjectId",
                   i.kind_id       AS "kind_id!: htui_core::model::ItemKindId",
                   i.key_prefix    AS "key_prefix!",
                   i.key_number    AS "key_number!",
                   i.key           AS "key!",
                   i.title         AS "title!",
                   i.body          AS "body!",
                   i.status        AS "status!: Status",
                   i.priority      AS "priority!",
                   i.required_tags AS "required_tags!",
                   i.touched_paths AS "touched_paths!",
                   i.step_graph_id AS "step_graph_id?: htui_core::model::StepGraphId",
                   i.version       AS "version!",
                   i.created_by    AS "created_by!: htui_core::model::UserId",
                   i.created_at    AS "created_at!",
                   i.updated_at    AS "updated_at!",
                   i.closed_at     AS "closed_at?",
                   i.resolution    AS "resolution?: htui_core::model::Resolution"
              FROM i, r
            "#,
            new.id.as_uuid(),
            new.project_id.as_uuid(),
            new.kind_id.as_uuid(),
            new.title,
            new.body,
            new.priority,
            &new.required_tags[..],
            &new.touched_paths[..],
            new.step_graph_id.map(|id| id.as_uuid()),
            new.created_by.as_uuid(),
            new.box_id.map(|id| id.as_uuid()),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        minted
            .ok_or_else(|| StoreError::Constraint(kind_not_in_project(new.kind_id, new.project_id)))
    }

    /// Compare-and-set edit over the seven spec columns of ANA-9 §4.2, with `COALESCE` patch
    /// semantics (ANA-9 §7.2).
    ///
    /// `step_graph_id` needs **two** parameters rather than a `COALESCE`: the field is a
    /// double-[`Option`], and `None` ("leave it") and `Some(None)` ("clear the override back to the
    /// kind default") would both arrive as SQL `NULL`. `CASE WHEN $9 THEN $10 END` splits them.
    ///
    /// `key_prefix`, `key_number` and `key` are never in the `SET` list, even across a kind change
    /// (§4.1), and `updated_at` is left to the trigger.
    ///
    /// Zero rows means one of three things, disambiguated by one follow-up read rather than by
    /// three statements up front: no such item, a lost race, or the kind guard. The version still
    /// matching after a zero-row compare-and-set can only be the guard, which is also `MemStore`'s
    /// order of judgements (compare-and-set first, kind check second).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the item does not exist, or when the ancestor revision a
    /// divergence needs is missing; [`StoreError::Constraint`] when the patched kind belongs to
    /// another project, or when `author_id` names no `app_user` row.
    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> Result<UpdateOutcome> {
        let updated = sqlx::query_as!(
            Item,
            r#"
            WITH u AS (
                UPDATE item SET
                    title         = COALESCE($3, title),
                    body          = COALESCE($4, body),
                    kind_id       = COALESCE($5, kind_id),
                    required_tags = COALESCE($6, required_tags),
                    priority      = COALESCE($7, priority),
                    touched_paths = COALESCE($8, touched_paths),
                    step_graph_id = CASE WHEN $9 THEN $10 ELSE step_graph_id END,
                    version       = version + 1
                 WHERE id = $1
                   AND version = $2
                   AND ($5::uuid IS NULL OR EXISTS (
                           SELECT 1 FROM item_kind k
                            WHERE k.id = $5 AND k.project_id = item.project_id))
                RETURNING id, project_id, kind_id, key_prefix, key_number, key, title, body, status,
                          priority, required_tags, touched_paths, step_graph_id, version,
                          created_by, created_at, updated_at, closed_at, resolution
            ), r AS (
                INSERT INTO item_revision (item_id, version, title, body, required_tags,
                                           author_id, box_id, reason)
                SELECT id, version, title, body, required_tags, $11, $12, $13 FROM u
                RETURNING item_id
            )
            SELECT u.id            AS "id!: ItemId",
                   u.project_id    AS "project_id!: htui_core::model::ProjectId",
                   u.kind_id       AS "kind_id!: htui_core::model::ItemKindId",
                   u.key_prefix    AS "key_prefix!",
                   u.key_number    AS "key_number!",
                   u.key           AS "key!",
                   u.title         AS "title!",
                   u.body          AS "body!",
                   u.status        AS "status!: Status",
                   u.priority      AS "priority!",
                   u.required_tags AS "required_tags!",
                   u.touched_paths AS "touched_paths!",
                   u.step_graph_id AS "step_graph_id?: htui_core::model::StepGraphId",
                   u.version       AS "version!",
                   u.created_by    AS "created_by!: htui_core::model::UserId",
                   u.created_at    AS "created_at!",
                   u.updated_at    AS "updated_at!",
                   u.closed_at     AS "closed_at?",
                   u.resolution    AS "resolution?: htui_core::model::Resolution"
              FROM u, r
            "#,
            id.as_uuid(),
            expected_version,
            patch.title,
            patch.body,
            patch.kind_id.map(|id| id.as_uuid()),
            patch.required_tags.as_deref(),
            patch.priority,
            patch.touched_paths.as_deref(),
            patch.step_graph_id.is_some(),
            patch.step_graph_id.flatten().map(|id| id.as_uuid()),
            patch.author_id.as_uuid(),
            patch.box_id.map(|id| id.as_uuid()),
            patch.reason,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if let Some(head) = updated {
            return Ok(UpdateOutcome::Updated(head));
        }

        let Some(head) = self.item(id).await? else {
            return Err(StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            });
        };
        if head.version == expected_version {
            // The compare-and-set matched, so the only other conjunct of the `WHERE` - the kind
            // guard - is what refused the row. It cannot have fired without a patched kind, and
            // `version` never decreases, so the `None` arm is unreachable rather than a second
            // rule: it is reported as a backend fault instead of being dressed up as a kind error.
            return Err(match patch.kind_id {
                Some(kind) => StoreError::Constraint(kind_not_in_project(kind, head.project_id)),
                None => StoreError::Backend(format!(
                    "item `{id}` refused a compare-and-set at version {expected_version} that \
                     matched: no rule of §4.2 can produce this"
                )),
            });
        }

        let ancestor = self.revision(id, expected_version).await?;
        Ok(UpdateOutcome::Diverged { head, ancestor })
    }

    /// The status compare-and-set of ANA-9 §4.2: never bumps `version`, never writes a revision.
    ///
    /// `closed_at` follows the *current* status in both directions - set on a move to a terminal
    /// status, cleared on a move back to a live one (blueprint H.12), which is the MOD-1 errata
    /// rule `closed_at = to.is_terminal().then_some(now)` written in SQL. [`legal_move`] refuses
    /// every move to `closed` (MOD-38 PRD D1; close-out is the only way in), so the one terminal
    /// status this `UPDATE` can reach is `done`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the item does not exist. A `from` that does not match the
    /// current status is `Ok(false)`, not an error, exactly as `MemStore` answers.
    ///
    /// [`StoreError::Constraint`] when `(from, to)` is outside the §4.3 table, decided in
    /// `htui-core` by [`legal_move`] and **before** the `UPDATE`, so a refused pair leaves the row
    /// byte-identical (plan D15). The `NotFound` above still wins on an unknown id (plan D14),
    /// which is what the existence probe in that branch is for.
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        if let Err(refusal) = legal_move(from, to) {
            let exists = sqlx::query_scalar!("SELECT 1 FROM item WHERE id = $1", id.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?
                .is_some();
            return refuse_illegal_move::<Status, _>(exists, id, refusal);
        }

        let moved = sqlx::query!(
            "UPDATE item \
                SET status    = $3, \
                    closed_at = CASE WHEN $3 = 'done' THEN clock_timestamp() ELSE NULL END \
              WHERE id = $1 AND status = $2",
            id.as_uuid(),
            from.as_str(),
            to.as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }

        let exists = sqlx::query_scalar!("SELECT 1 FROM item WHERE id = $1", id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_some();

        if exists {
            Ok(false)
        } else {
            Err(StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            })
        }
    }

    /// Appends session events in one statement under `fence`, skipping the `(run_step_id, seq)`
    /// pairs already stored (ANA-9 §4.3, plan D3; MOD-40 plan D1).
    ///
    /// The batch is carried as **one** `jsonb` parameter and expanded by `jsonb_to_recordset`,
    /// rather than as nine parallel arrays: `SessionEvent`'s serde form already *is* the row -
    /// field names are the column names verbatim - and nullable `text[]` / `jsonb[]` parameters
    /// are avoided entirely.
    ///
    /// The fence is decided **inside** the statement (MOD-40 blueprint B1, B2). `lease`
    /// key-share-locks each named step and then share-locks its run (`FOR KEY SHARE OF s FOR SHARE
    /// OF r`, MOD-77 plan D3, review L1: `park_step`'s order, so a park holding the step cannot
    /// deadlock against the insert's foreign-key lock on it; `KEY SHARE` is that lock, so the
    /// append does not wait behind the step's own usage, settle or transition writes), so a
    /// `take_lease` or `adopt_runs` either waits for this write or is seen by it; `fenced` is the
    /// lowest step whose run's `lease_owner` is not `$2`. The insert runs only when nothing is
    /// fenced, **or** when a named step does not exist at all, so that such a row still reaches the
    /// foreign key and refuses the batch (`23503`) ahead of the fence. The statement answers both
    /// facts, so no second read is needed and none can race.
    ///
    /// A batch naming two steps of one run may still lock them in either order against a park of
    /// the second (MOD-77 plan D7); the only production caller, `Recorder::flush`, writes one
    /// step per batch.
    ///
    /// `inserted` counts inserts only, because `ON CONFLICT ... DO NOTHING` skips a stored row: that
    /// is the "how many landed" answer §4.1 asks for, and what tells a recorder's replay (short is
    /// fine) from a fresh batch (short is a second writer).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when an event names a `run_step` that does not exist (`23503`)
    /// or a `kind` / `role` outside the §4.3 `CHECK` lists (`23514`); [`StoreError::Fenced`] when a
    /// named step's run does not carry `fence`'s lease. One statement, so a refused batch writes
    /// none of its rows.
    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let rows = serde_json::to_value(events).map_err(|err| {
            StoreError::Backend(format!("session_event does not serialise: {err}"))
        })?;

        let answer = sqlx::query!(
            r#"
            WITH e AS (
                SELECT *
                  FROM jsonb_to_recordset($1::jsonb)
                       AS e(run_step_id uuid, seq int, turn int, kind text, role text,
                            tool_call_id text, payload jsonb, raw jsonb, at timestamptz)
            ),
            lease AS (
                SELECT s.id AS run_step_id, r.lease_owner
                  FROM run_step s
                  JOIN run r ON r.id = s.run_id
                 WHERE s.id IN (SELECT run_step_id FROM e)
                   FOR KEY SHARE OF s FOR SHARE OF r
            ),
            fenced AS (
                SELECT run_step_id
                  FROM lease
                 WHERE lease_owner IS DISTINCT FROM $2
                 ORDER BY run_step_id
                 LIMIT 1
            ),
            ins AS (
                INSERT INTO session_event (run_step_id, seq, turn, kind, role, tool_call_id,
                                           payload, raw, at)
                SELECT e.run_step_id, e.seq, e.turn, e.kind, e.role, e.tool_call_id, e.payload,
                       e.raw, e.at
                  FROM e
                 WHERE NOT EXISTS (SELECT 1 FROM fenced)
                    OR EXISTS (SELECT 1 FROM e AS m
                                WHERE NOT EXISTS (SELECT 1 FROM lease l
                                                   WHERE l.run_step_id = m.run_step_id))
                ON CONFLICT (run_step_id, seq) DO NOTHING
                RETURNING 1
            )
            SELECT (SELECT count(*) FROM ins) AS "inserted!",
                   (SELECT run_step_id FROM fenced) AS "fenced_step?"
            "#,
            rows,
            fence.owner(),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if let Some(step) = answer.fenced_step {
            return Err(StoreError::Fenced {
                step: StepId::from_uuid(step),
            });
        }
        Ok(usize::try_from(answer.inserted).unwrap_or(0))
    }

    /// `run_step.usage` on every call, `run_step.prompt_digest` only when one is supplied
    /// (`docs/ANA-4.md` §4.1, plan D15(b)).
    ///
    /// `COALESCE($3, prompt_digest)` is what makes `None` mean "leave it": the recorder computes
    /// the digest once, at the prompt, and every later usage write for the same step passes `None`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the step does not exist and [`StoreError::Fenced`] when it
    /// does and its run does not carry `fence`'s lease, told apart by `step_exists`' follow-up read
    /// on a miss (`interrupt_step`'s shape). The fence is a locking CTE (MOD-77 plan D2): the step
    /// `FOR NO KEY UPDATE`, then its run `FOR SHARE`. The run lock is `append_events`' reason
    /// (MOD-40 blueprint B2). Holding the step first is `park_step`'s order, so a park on the same
    /// step cannot deadlock against this write (`40P01`,
    /// `pg_criteria.rs::set_step_usage_takes_the_step_first`), and the step's update lock is taken
    /// up front rather than upgraded. The clause order is load-bearing.
    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        let updated = sqlx::query!(
            "WITH locked AS ( \
                 SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
                  WHERE s.id = $1 \
                    AND r.lease_owner IS NOT DISTINCT FROM $4 \
                    FOR NO KEY UPDATE OF s FOR SHARE OF r) \
             UPDATE run_step \
                SET usage = $2, prompt_digest = COALESCE($3, prompt_digest) \
               FROM locked \
              WHERE run_step.id = locked.id",
            step.as_uuid(),
            usage,
            prompt_digest.as_deref(),
            fence.owner(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if updated == 1 {
            return Ok(());
        }
        Err(Box::pin(fenced_or_missing(&self.pool, step)).await)
    }

    /// Creates or edits one `agent` row as a compare-and-set on `updated_at` (`docs/ANA-4.md`
    /// §4.1, ANA-9 §5.7; MOD-40 plan D5).
    ///
    /// Two statements, one per branch, `set_setting`'s `App` rung's shape: `expected: None` is
    /// `INSERT … ON CONFLICT (id) DO NOTHING`, so a stored id is a miss and never an overwrite —
    /// and the `id` arbiter is checked before the name's unique index, so a stored id is `Stale`
    /// even under a held name; `Some(t)` is `UPDATE … WHERE id = $1 AND updated_at = $t`, whose
    /// `SET` list names neither stamp: `created_at` stays the insert's and the `BEFORE UPDATE`
    /// trigger writes `updated_at`, which `RETURNING` sees. A spent token matches no row, so no
    /// unique check runs and a stale rename is `Stale`, not `Constraint` (blueprint P-7). A miss
    /// reads the row once ([`cas_miss`]): present is `Stale`, absent is `NotFound`.
    ///
    /// The insert keeps the caller's stamps at the column's resolution, microseconds; `Applied`
    /// carries them as stored, which is the token the next edit passes.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for `Some(_)` on an id no row has; [`StoreError::Constraint`]
    /// when another id holds the name (`23505` on `agent_name_key`).
    async fn upsert_agent(
        &self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Agent>> {
        let landed = match expected {
            None => sqlx::query_as!(
                Agent,
                r#"
                INSERT INTO agent (id, name, transport, launch, models, default_model, billing,
                                   enabled, settings, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                ON CONFLICT (id) DO NOTHING
                RETURNING id            AS "id: AgentId",
                          name,
                          transport     AS "transport: htui_core::model::Transport",
                          launch,
                          models,
                          default_model,
                          billing       AS "billing: htui_core::model::Billing",
                          enabled,
                          settings,
                          created_at,
                          updated_at
                "#,
                agent.id.as_uuid(),
                agent.name,
                agent.transport.as_str(),
                &agent.launch,
                &agent.models[..],
                agent.default_model.as_deref(),
                agent.billing.as_str(),
                agent.enabled,
                &agent.settings,
                agent.created_at,
                agent.updated_at,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
            Some(token) => sqlx::query_as!(
                Agent,
                r#"
                UPDATE agent
                   SET name          = $2,
                       transport     = $3,
                       launch        = $4,
                       models        = $5,
                       default_model = $6,
                       billing       = $7,
                       enabled       = $8,
                       settings      = $9
                 WHERE id = $1 AND updated_at = $10
                RETURNING id            AS "id: AgentId",
                          name,
                          transport     AS "transport: htui_core::model::Transport",
                          launch,
                          models,
                          default_model,
                          billing       AS "billing: htui_core::model::Billing",
                          enabled,
                          settings,
                          created_at,
                          updated_at
                "#,
                agent.id.as_uuid(),
                agent.name,
                agent.transport.as_str(),
                &agent.launch,
                &agent.models[..],
                agent.default_model.as_deref(),
                agent.billing.as_str(),
                agent.enabled,
                &agent.settings,
                token,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
        };
        match landed {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => cas_miss(self.stored_agent(agent.id).await?, "agent", agent.id),
        }
    }

    /// Inserts or updates one `agent_box` row on its composite primary key (`docs/ANA-4.md` §4.1,
    /// ANA-9 §5.7).
    ///
    /// `quota` and `quota_at` are in **neither** list (MOD-2 plan D74): not in the `INSERT`
    /// column list, so a fresh row gets the column default `NULL` - the honest value, since a row
    /// nobody has latched has no observed allowance and a probe handshake reports none (§7) - and
    /// not in the `DO UPDATE SET` list, so an upsert of an existing row cannot touch them.
    /// [`WriteStore::set_agent_box_quota`] is their only writer.
    ///
    /// A `row` carrying either field is therefore accepted and ignored, which is the one sharp
    /// edge this design keeps: it is not an error and it is not a write. That is deliberate.
    /// Before D74 the probe read the row at chat start and handed both columns back to this
    /// statement seconds later, discarding every latch that had landed in between - a lost update
    /// by construction. `COALESCE(EXCLUDED.quota, agent_box.quota)` was considered and rejected:
    /// it would still let an upsert *set* the column, and would make clearing it impossible.
    ///
    /// Since MOD-23 (plan D242) the conflict arm writes `enabled = EXCLUDED.enabled AND NOT
    /// agent_box.user_off`: a row the human switched off on this box stays off whatever the probe
    /// proposes, while every other column the probe owns is still written. The insert is
    /// unchanged, since a fresh row takes `user_off`'s default `false`.
    /// [`WriteStore::set_agent_box_enabled`] is the only writer of `user_off`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when the agent or the box does not exist (`23503`).
    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()> {
        sqlx::query!(
            "INSERT INTO agent_box (agent_id, box_id, enabled, version, path, probed_at, \
                                    updated_at, probe) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (agent_id, box_id) DO UPDATE SET \
                 enabled   = EXCLUDED.enabled AND NOT agent_box.user_off, \
                 version   = EXCLUDED.version, \
                 path      = EXCLUDED.path, \
                 probed_at = EXCLUDED.probed_at, \
                 probe     = EXCLUDED.probe",
            row.agent_id.as_uuid(),
            row.box_id.as_uuid(),
            row.enabled,
            row.version.as_deref(),
            row.path.as_deref(),
            row.probed_at,
            row.updated_at,
            row.probe.as_ref(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// The two-column quota latch of `docs/ANA-4.md` §7 (MOD-2 plan D67), keyed on the composite
    /// primary key, newest `quota_at` winning (MOD-40 plan D4).
    ///
    /// Two columns and no more: `probe` and the four discovery columns belong to the probe, which
    /// may be re-running beside this write. `updated_at` is the migration's `BEFORE UPDATE`
    /// trigger's - `agent_box` is in the `0001_init.sql:577` loop, and "no write path may set
    /// `updated_at` by hand" is that loop's own rule.
    ///
    /// One statement answers both facts the trait tells apart (MOD-40 blueprint B11): `written`
    /// from the guarded `UPDATE`, and `present` from the same statement's snapshot, so "absent"
    /// and "a newer quota is stored" can never be confused by a row that appears between two
    /// reads. Under `READ COMMITTED` the `UPDATE` re-checks its `WHERE` on the newest version of
    /// a row a concurrent latch just committed, so of two racing latches the older one loses.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when no `agent_box` row has that key.
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<bool> {
        let verdict = sqlx::query!(
            r#"
            WITH latched AS (
                UPDATE agent_box
                   SET quota = $3, quota_at = $4
                 WHERE agent_id = $1 AND box_id = $2
                   AND (quota_at IS NULL OR quota_at <= $4)
                RETURNING 1
            )
            SELECT EXISTS (SELECT 1 FROM latched) AS "written!",
                   EXISTS (SELECT 1 FROM agent_box
                            WHERE agent_id = $1 AND box_id = $2) AS "present!"
            "#,
            agent_id.as_uuid(),
            box_id.as_uuid(),
            &quota,
            quota_at,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if !verdict.present {
            return Err(StoreError::NotFound {
                entity: "agent_box",
                id: format!("{agent_id}/{box_id}"),
            });
        }
        Ok(verdict.written)
    }

    /// The per-box switch (MOD-23 D242, blueprint D249): one statement, keyed on the composite
    /// primary key. `user_off` and `enabled` and nothing else; `updated_at` is the `BEFORE UPDATE`
    /// trigger's on the conflict arm and the column default on the insert.
    ///
    /// Switching on re-derives `enabled` from the stored probe. The `COALESCE` is load-bearing: a
    /// probe document without `status` makes `->>'status'` `NULL`, and `true AND NULL` would
    /// violate `enabled NOT NULL` (blueprint F-1).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when the agent or the box does not exist (`23503`).
    async fn set_agent_box_enabled(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        enabled: bool,
    ) -> Result<()> {
        sqlx::query!(
            "INSERT INTO agent_box (agent_id, box_id, enabled, user_off) \
             VALUES ($1, $2, $3, NOT $3) \
             ON CONFLICT (agent_id, box_id) DO UPDATE SET \
                 user_off = NOT $3, \
                 enabled  = $3 AND (agent_box.probe IS NULL \
                                    OR COALESCE(agent_box.probe->>'status' = 'ready', false))",
            agent_id.as_uuid(),
            box_id.as_uuid(),
            enabled,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// One box probe in one transaction (MOD-7 D10): the nine probe columns of the row, then the
    /// whole `box_tool` set deleted and re-inserted.
    ///
    /// The `UPDATE` names no column a person or registration owns, and not `updated_at`: the
    /// `BEFORE UPDATE` trigger moves it. A tool name listed twice is the `box_tool` primary key's
    /// `23505` and a malformed digest is `0005`'s `CHECK` (`23514`); [`map_sqlx`] makes both a
    /// [`StoreError::Constraint`], and the dropped transaction rolls the row back with them.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] with `entity: "box"` when no row has `probe.box_id`;
    /// [`StoreError::Constraint`] as above.
    async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let updated = sqlx::query!(
            r#"
            UPDATE box
               SET os_version = $2, cpu = $3, ram_mb = $4, gpu_present = $5, gpu_vendor = $6,
                   probed_tags = $7, htui_version = $8, last_probed_at = $9, probe_spec_digest = $10
             WHERE id = $1
            RETURNING id AS "id!: BoxId"
            "#,
            probe.box_id.as_uuid(),
            probe.os_version,
            probe.cpu,
            probe.ram_mb,
            probe.gpu_present,
            probe.gpu_vendor.as_deref(),
            &probe.probed_tags,
            probe.htui_version,
            probe.probed_at,
            probe.spec_digest,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if updated.is_none() {
            return Err(StoreError::NotFound {
                entity: "box",
                id: probe.box_id.to_string(),
            });
        }

        sqlx::query!(
            "DELETE FROM box_tool WHERE box_id = $1",
            probe.box_id.as_uuid()
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let names: Vec<String> = probe.tools.iter().map(|tool| tool.name.clone()).collect();
        let versions: Vec<String> = probe
            .tools
            .iter()
            .map(|tool| tool.version.clone())
            .collect();
        let paths: Vec<String> = probe.tools.iter().map(|tool| tool.path.clone()).collect();
        sqlx::query!(
            r#"
            INSERT INTO box_tool (box_id, name, version, path, probed_at)
            SELECT $1, t.name, t.version, t.path, $5
              FROM UNNEST($2::text[], $3::text[], $4::text[]) AS t(name, version, path)
            "#,
            probe.box_id.as_uuid(),
            &names,
            &versions,
            &paths,
            probe.probed_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)
    }

    /// Every box of this user, by id, each with its `box_tool` rows in name byte order
    /// (`COLLATE "C"`, the order `MemStore` sorts by) and `probe_spec_digest` (MOD-7 D10, D18).
    ///
    /// Two reads rather than a `query_as!(BoxRow, ..)`: the digest is not a [`BoxRow`] field, so
    /// the rows are mapped by hand.
    ///
    /// Both reads share one snapshot: they run in one `begin_repeatable_read` transaction, so a
    /// box inserted or a probe committed between them cannot hand back a box whose tools are
    /// missing or a tool list from after the box row was read. Read-only, so no `40001` can arise
    /// and nothing is retried.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn boxes(&self) -> Result<Vec<BoxRecord>> {
        let mut tx = begin_repeatable_read(&self.pool).await?;
        let rows = sqlx::query!(
            r#"
            SELECT id AS "id: BoxId", user_id AS "user_id: htui_core::model::UserId", hostname,
                   os_family AS "os_family: htui_core::model::OsFamily", os_version, arch, cpu,
                   ram_mb, gpu_present, gpu_vendor, htui_version, probed_tags, declared_tags,
                   quirks, settings, registered_at, last_seen_at, last_probed_at, updated_at,
                   edit_version, probe_spec_digest
              FROM box WHERE user_id = $1 ORDER BY id
            "#,
            self.this_user().as_uuid(),
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let ids: Vec<Uuid> = rows.iter().map(|row| row.id.as_uuid()).collect();
        let tools = sqlx::query!(
            r#"
            SELECT box_id AS "box_id: BoxId", name, version, path, probed_at
              FROM box_tool WHERE box_id = ANY($1) ORDER BY box_id, name COLLATE "C"
            "#,
            &ids,
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        let mut tools = tools
            .into_iter()
            .map(|tool| BoxTool {
                box_id: tool.box_id,
                name: tool.name,
                version: tool.version,
                path: tool.path,
                probed_at: tool.probed_at,
            })
            .peekable();

        Ok(rows
            .into_iter()
            .map(|row| {
                let mut own = Vec::new();
                while let Some(tool) = tools.next_if(|tool| tool.box_id == row.id) {
                    own.push(tool);
                }
                BoxRecord {
                    row: BoxRow {
                        id: row.id,
                        user_id: row.user_id,
                        hostname: row.hostname,
                        os_family: row.os_family,
                        os_version: row.os_version,
                        arch: row.arch,
                        cpu: row.cpu,
                        ram_mb: row.ram_mb,
                        gpu_present: row.gpu_present,
                        gpu_vendor: row.gpu_vendor,
                        htui_version: row.htui_version,
                        probed_tags: row.probed_tags,
                        declared_tags: row.declared_tags,
                        quirks: row.quirks,
                        settings: row.settings,
                        registered_at: row.registered_at,
                        last_seen_at: row.last_seen_at,
                        last_probed_at: row.last_probed_at,
                        updated_at: row.updated_at,
                        edit_version: row.edit_version,
                    },
                    tools: own,
                    probe_spec_digest: row.probe_spec_digest,
                }
            })
            .collect())
    }

    /// The editors' compare-and-set (MOD-7 D41, D58): one `UPDATE` with the user filter and the
    /// token in its `WHERE`, `COALESCE` so a `None` field keeps its column. A miss re-reads the row
    /// through [`PgStore::box_row`], kept only when it is this user's, and `cas_miss` decides
    /// `Stale` or `NotFound`. An invalid tag list takes the same read first, so `NotFound` and
    /// `Stale` win over `Constraint` (the order `update_item_kind` keeps).
    ///
    /// No `updated_at` is written here: the `set_updated_at` trigger stamps it.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown box or another user's, `Constraint` for a refused tag, and
    /// whatever the driver reports, through [`map_sqlx`].
    async fn edit_box(
        &self,
        id: BoxId,
        expected: i32,
        edit: BoxEdit,
    ) -> Result<CasOutcome<BoxRow>> {
        let me = self.this_user();
        let executor = edit.executor.as_ref();
        let tags = edit
            .declared_tags
            .as_deref()
            .map(canonical_declared_tags)
            .transpose();
        // MOD-41 blueprint B-5: a refusal known before the statement reads the row first, so
        // `NotFound` and `Stale` keep their precedence over it (MOD-7 D41, plan D10).
        let refused = match (&tags, executor) {
            (Err(sentence), _) => Some(sentence.clone()),
            (Ok(_), Some(Executor::Other(_))) => Some(EXECUTOR_MUST_BE_KNOWN.to_owned()),
            _ => None,
        };
        if let Some(sentence) = refused {
            let current = self.box_row(id).await?.filter(|row| row.user_id == me);
            return match current {
                None => Err(StoreError::NotFound {
                    entity: "box",
                    id: id.to_string(),
                }),
                Some(row) if row.edit_version != expected => Ok(CasOutcome::Stale(row)),
                Some(_) => Err(StoreError::Constraint(sentence)),
            };
        }
        let tags = tags.expect("a refused tag list returned above");
        // `jsonb_set` errors (`22023`) on a blob that is not an object, so the `WHERE` refuses one
        // instead and the miss below names it: race-safe, one statement (blueprint B-5).
        let written = sqlx::query_as!(
            BoxRow,
            r#"
            UPDATE box
               SET declared_tags = COALESCE($3, declared_tags),
                   quirks        = COALESCE($4, quirks),
                   settings      = CASE WHEN $6::text IS NULL THEN settings
                                        ELSE jsonb_set(settings, '{executor}', to_jsonb($6::text))
                                   END,
                   edit_version  = edit_version + 1
             WHERE id = $1 AND user_id = $5 AND edit_version = $2
               AND ($6::text IS NULL OR jsonb_typeof(settings) = 'object')
            RETURNING id             AS "id: BoxId",
                      user_id        AS "user_id: htui_core::model::UserId",
                      hostname,
                      os_family      AS "os_family: htui_core::model::OsFamily",
                      os_version, arch, cpu, ram_mb, gpu_present, gpu_vendor, htui_version,
                      probed_tags, declared_tags, quirks, settings,
                      registered_at, last_seen_at, last_probed_at, updated_at, edit_version
            "#,
            id.as_uuid(),
            expected,
            tags.as_deref(),
            edit.quirks,
            me.as_uuid(),
            executor.map(Executor::as_str),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        match written {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => {
                let current = self.box_row(id).await?.filter(|row| row.user_id == me);
                match current {
                    Some(row)
                        if row.edit_version == expected
                            && executor.is_some()
                            && !row.settings.is_object() =>
                    {
                        Err(StoreError::Constraint(
                            BOX_SETTINGS_NOT_AN_OBJECT.to_owned(),
                        ))
                    }
                    other => cas_miss(other, "box", id),
                }
            }
        }
    }

    /// The overlay row (MOD-51 D2): `PgStore::stored_setting`'s `App` statement, byte for byte,
    /// bound to [`BOX_PROBE_SPEC_KEY`], so it reuses that statement's offline entry.
    async fn box_probe_spec(&self) -> Result<Option<StoredSetting>> {
        let row = sqlx::query!(
            "SELECT value, updated_at FROM app_setting WHERE key = $1",
            BOX_PROBE_SPEC_KEY,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(row.map(|row| StoredSetting {
            value: Some(row.value),
            updated_at: row.updated_at,
        }))
    }

    /// The overlay's compare-and-set (MOD-51 D2): the `App` rung's three statements of
    /// `set_setting` and `clear_setting`, byte for byte, so no offline entry is added. A miss
    /// re-reads through [`box_probe_spec`](WriteStore::box_probe_spec) and answers `Stale` with
    /// whatever is there now, `None` included; unlike `cas_miss`, never `NotFound`.
    ///
    /// The re-read is a second statement outside the write, the window every `cas_miss` caller
    /// has: a row inserted or deleted between the two is reported as it is at the re-read.
    async fn set_box_probe_spec(
        &self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredSetting>>> {
        if overlay.as_ref().is_some_and(|value| !value.is_object()) {
            return Err(StoreError::Constraint(
                BOX_PROBE_SPEC_NOT_AN_OBJECT.to_owned(),
            ));
        }
        let name = BOX_PROBE_SPEC_KEY;
        // The three literals are `set_setting`'s and `clear_setting`'s `App` statements byte for
        // byte, indentation included: the offline cache keys a statement by the SHA-256 of its
        // text (MOD-51 F-9), so they look mis-indented here on purpose.
        let landed: Option<DateTime<Utc>> = match (&overlay, expected) {
            // Before any SQL, so both refusals win over `Stale` (D2's precedence).
            (None, None) => {
                return Err(StoreError::Constraint(
                    BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN.to_owned(),
                ));
            }
            // "I expect no row": a conflict means somebody is there, and that is `Stale`.
            (Some(value), None) => sqlx::query_scalar!(
                r#"
                        INSERT INTO app_setting (key, value) VALUES ($1, $2)
                        ON CONFLICT (key) DO NOTHING
                        RETURNING updated_at AS "updated_at!"
                        "#,
                name,
                value,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
            (Some(value), Some(token)) => sqlx::query_scalar!(
                r#"
                        UPDATE app_setting SET value = $2
                         WHERE key = $1 AND updated_at = $3
                        RETURNING updated_at AS "updated_at!"
                        "#,
                name,
                value,
                token,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
            (None, Some(token)) => sqlx::query_scalar!(
                r#"
                    DELETE FROM app_setting WHERE key = $1 AND updated_at = $2
                    RETURNING updated_at AS "updated_at!"
                    "#,
                name,
                token,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
        };
        match landed {
            Some(updated_at) => Ok(CasOutcome::Applied(overlay.map(|value| StoredSetting {
                value: Some(value),
                updated_at,
            }))),
            None => Ok(CasOutcome::Stale(self.box_probe_spec().await?)),
        }
    }

    /// The `run` / `run_step` pair of a free-standing chat, in one transaction, both
    /// `ON CONFLICT (id) DO NOTHING` (plan D4).
    ///
    /// The pair is `kind 'chat'`, `mode 'manual'`, `item_id NULL`, the spec's `phase_name`
    /// (`'chat'`, or `'edit_help'` for a help turn, MOD-55 plan P5), `position 0`, `attempt 1`,
    /// `fanout_index 0`, `executing_box_id` = the target box, both rows
    /// `running` with `finished_at` NULL (which [`WriteStore::finish_chat_run`] closes), `agent_id`
    /// and `model` from the spec, and every stamp `chat.started_at`. The ids are minted
    /// client-side, so a start retried after an answer that never arrived lands on the pair the
    /// first attempt wrote rather than colliding (ANA-9 §4.3). Nothing else inserts the pair: the
    /// offline buffer that once uploaded the same chat is gone (MOD-25, MOD-40), and a step write
    /// the store refuses is re-offered by the recorder itself.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when the project, box, user or agent does not exist (`23503`).
    async fn start_chat_run(&self, chat: &ChatRunSpec) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        sqlx::query!(
            "INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, \
                              executing_box_id, started_by, queued_at, started_at, finished_at) \
             VALUES ($1, $2, NULL, 'chat', 'manual', 'running', $3, $3, $4, $5, $5, NULL) \
             ON CONFLICT (id) DO NOTHING",
            chat.run_id.as_uuid(),
            chat.project_id.as_uuid(),
            chat.target_box_id.as_uuid(),
            chat.started_by.as_uuid(),
            chat.started_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query!(
            "INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, \
                                   agent_id, model, status, started_at, finished_at) \
             VALUES ($1, $2, 0, 1, 0, $3, $4, $5, 'running', $6, NULL) \
             ON CONFLICT (id) DO NOTHING",
            chat.step_id.as_uuid(),
            chat.run_id.as_uuid(),
            chat.phase_name.as_str(),
            chat.agent_id.map(AgentId::as_uuid),
            chat.model.as_deref(),
            chat.started_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)
    }

    /// Closes both rows of a chat run in one transaction (plan D4, assumption A1).
    ///
    /// Without it `active_runs` would count a finished chat forever. The step takes the status of
    /// the same name; the three values `RunStatus` and `StepStatus` share are the only ones
    /// accepted, so a live status is refused before either row is touched rather than leaving the
    /// pair half-closed.
    ///
    /// MOD-77 plan D5: the step is locked `FOR NO KEY UPDATE` before the run is updated. A chat's
    /// `set_step_usage` locks the step and then the run; closing the run first and the step second
    /// would close a cycle against it (`40P01`,
    /// `pg_criteria.rs::finish_chat_run_takes_the_step_first`). Errors keep their order: the run's
    /// `NotFound` before the step's.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a non-terminal `status`; [`StoreError::NotFound`] when the
    /// run or the step does not exist.
    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> Result<()> {
        let step_status = chat_step_status(status)
            .ok_or_else(|| StoreError::Constraint(not_a_terminal_status(status)))?;

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        // MOD-77 plan D5: the step first, so this close and a usage write on the same chat lock in
        // one order. Absence is remembered, not answered yet: an unknown run is `NotFound { run }`
        // before an unknown step (conformance `start_chat_run_mints_chat_rows`).
        let step_found = sqlx::query_scalar!(
            "SELECT 1 FROM run_step WHERE id = $1 FOR NO KEY UPDATE",
            step.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .is_some();

        let closed_run = sqlx::query!(
            "UPDATE run SET status = $2, finished_at = $3 WHERE id = $1",
            run.as_uuid(),
            status.as_str(),
            finished_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        if closed_run == 0 {
            return Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            });
        }
        if !step_found {
            return Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            });
        }

        let closed_step = sqlx::query!(
            "UPDATE run_step SET status = $2, finished_at = $3 WHERE id = $1",
            step.as_uuid(),
            step_status.as_str(),
            finished_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        // Unreachable under the up-front step lock (a held row cannot be deleted); kept as a
        // defensive check.
        if closed_step == 0 {
            return Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            });
        }

        tx.commit().await.map_err(map_sqlx)
    }

    /// `run_step.prompt_digest` and `run_step.trim_record`, both, unconditionally
    /// (`docs/ANA-5.md` §4.4): the pre-flight audit of `R-PRM-3`, written at stage 3 before a
    /// session starts.
    ///
    /// No `COALESCE` here, unlike [`set_step_usage`](PgStore::set_step_usage) above: that one's
    /// digest is optional because the chat path writes usage many times and a digest once, while a
    /// caller of this one has just assembled a prompt and always has both values. A re-run of the
    /// same step assembles a new prompt, so overwriting is the behaviour, not a side effect.
    ///
    /// `updated_at` is not in the `SET` list: the migration's `BEFORE UPDATE` trigger owns it.
    ///
    /// Written only while the step's run carries `fence`'s lease: `set_step_usage`'s locking CTE,
    /// verbatim (MOD-41 plan D1, MOD-77 plan D2), the step and then its run.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the step does not exist and [`StoreError::Fenced`] when it
    /// does and its run does not carry `fence`'s lease, told apart by `step_exists`' follow-up
    /// read on a miss.
    async fn set_step_prompt(
        &self,
        fence: StepFence,
        step: StepId,
        digest: &str,
        trim: &Value,
    ) -> Result<()> {
        let updated = sqlx::query!(
            "WITH locked AS ( \
                 SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
                  WHERE s.id = $1 \
                    AND r.lease_owner IS NOT DISTINCT FROM $4 \
                    FOR NO KEY UPDATE OF s FOR SHARE OF r) \
             UPDATE run_step SET prompt_digest = $2, trim_record = $3 \
               FROM locked \
              WHERE run_step.id = locked.id",
            step.as_uuid(),
            digest,
            trim,
            fence.owner(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if updated == 1 {
            return Ok(());
        }
        // Boxed as `set_step_usage`'s miss is: a debug build's worker stack.
        Err(Box::pin(fenced_or_missing(&self.pool, step)).await)
    }

    // ---- MOD-15 milestone 1: the hierarchy (plan D1-D12) ---------------------------------------
    //
    // Twenty-one writers, `delete_reach`, and the nine readers of D1 - whose statements live in
    // `pg/read.rs` beside the other reads, because one trait has one `impl` block but a reader
    // belongs with readers. Every edit is `WHERE id = $1 AND updated_at = $2`; **no statement below
    // sets `updated_at`**, because `0001_init.sql`'s `BEFORE UPDATE` trigger owns it and the cache
    // cursor of §4.4 rides on what that trigger writes. `RETURNING` therefore sees the token the
    // caller must present next, without a second read.

    // workspace

    /// One `INSERT ... RETURNING`; the row comes back with the server's `now()`, not the caller's.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when `slug` is taken (`23505`) or `created_by` names no
    /// `app_user` row (`23503`, which is what the nil UUID does).
    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace> {
        sqlx::query_as!(
            Workspace,
            r#"
            INSERT INTO workspace (id, slug, name, description, created_by)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id         AS "id: WorkspaceId",
                      slug,
                      name,
                      description,
                      created_by AS "created_by: htui_core::model::UserId",
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.slug,
            new.name,
            new.description,
            new.created_by.as_uuid(),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The compare-and-set of D3 with `COALESCE` patch semantics, exactly as
    /// [`update_item`](PgStore::update_item) has them.
    ///
    /// A slug collision is the database's (`23505`) and arrives as [`StoreError::Constraint`]; a
    /// spent token matches no row and never reaches the unique index, so a stale edit into a taken
    /// slug answers `Stale` rather than `Constraint` — the same order of judgements `MemStore`
    /// makes.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] on a `slug`
    /// collision.
    async fn update_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
    ) -> Result<CasOutcome<Workspace>> {
        let updated = sqlx::query_as!(
            Workspace,
            r#"
            UPDATE workspace SET
                slug        = COALESCE($3, slug),
                name        = COALESCE($4, name),
                description = COALESCE($5, description)
             WHERE id = $1 AND updated_at = $2
            RETURNING id         AS "id: WorkspaceId",
                      slug,
                      name,
                      description,
                      created_by AS "created_by: htui_core::model::UserId",
                      created_at,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.slug,
            patch.name,
            patch.description,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        match updated {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => cas_miss(self.workspace_row(id).await?, "workspace", id),
        }
    }

    /// One workspace by id, `None` when there is no such row.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn workspace(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        self.workspace_row(id).await
    }

    // workspace links and box paths

    /// `ON CONFLICT (workspace_id, project_id) DO UPDATE SET position`: the table's primary key is
    /// the identity, so a second call repositions rather than inserting a second row. No CAS —
    /// `workspace_project` is not in the trigger loop and has no `updated_at` to compare (D3).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when either id names no row (`23503`).
    async fn upsert_workspace_project(&self, link: &WorkspaceProject) -> Result<()> {
        sqlx::query!(
            "INSERT INTO workspace_project (workspace_id, project_id, position) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (workspace_id, project_id) DO UPDATE SET position = EXCLUDED.position",
            link.workspace_id.as_uuid(),
            link.project_id.as_uuid(),
            link.position,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// Removes one link. The project survives it: the cascade runs from `project` to
    /// `workspace_project`, never the other way (`0001_init.sql:162`).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when no such link exists.
    async fn remove_workspace_project(
        &self,
        workspace: WorkspaceId,
        project: ProjectId,
    ) -> Result<()> {
        let removed = sqlx::query!(
            "DELETE FROM workspace_project WHERE workspace_id = $1 AND project_id = $2",
            workspace.as_uuid(),
            project.as_uuid(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if removed == 0 {
            return Err(StoreError::NotFound {
                entity: "workspace_project",
                id: format!("{workspace}/{project}"),
            });
        }
        Ok(())
    }

    /// A workspace's links ordered by `position`, then `project_id` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn workspace_projects(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceProject>> {
        self.workspace_project_rows(workspace).await
    }

    /// One row per `(workspace_id, box_id)`, replaced in place (`R-BOX-4`). `updated_at` is in
    /// neither list: the insert takes the column default and the `DO UPDATE` leaves it to the
    /// trigger.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when either id names no row (`23503`).
    async fn upsert_workspace_box_path(&self, path: &WorkspaceBoxPath) -> Result<()> {
        sqlx::query!(
            "INSERT INTO workspace_box_path (workspace_id, box_id, root_path) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (workspace_id, box_id) DO UPDATE SET root_path = EXCLUDED.root_path",
            path.workspace_id.as_uuid(),
            path.box_id.as_uuid(),
            path.root_path,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// Every box's root path for a workspace, ordered by `box_id` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn workspace_box_paths(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceBoxPath>> {
        self.workspace_box_path_rows(workspace).await
    }

    // project

    /// The project row and, in the same transaction, the thirty-five rows `seed_project` gives
    /// every project: five default graphs, their fifteen phases, five kinds and the ten
    /// `DEFAULT_TEMPLATES` at version 1 (M1 D9, M2 D4). `settings` takes the column default `{}`
    /// and the two secret columns are MOD-10's.
    ///
    /// The explicit transaction is what makes "a project that fails to seed never existed" true
    /// rather than aspirational; `item_key_counter` is not among the rows, because
    /// [`mint_item`](WriteStore::mint_item) creates that one on first use (M2 D5).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when `slug` is taken (`23505`) or `created_by` names no
    /// `app_user` row (`23503`) — both on the first statement, before any seed row.
    async fn create_project(&self, new: NewProject) -> Result<Project> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let created = sqlx::query_as!(
            Project,
            r#"
            INSERT INTO project (id, slug, name, description, created_by)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id              AS "id: ProjectId",
                      slug,
                      name,
                      description,
                      secret_provider,
                      secret_scope,
                      settings,
                      created_by      AS "created_by: htui_core::model::UserId",
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.slug,
            new.name,
            new.description,
            new.created_by.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        seed_project(&mut tx, created.id, created.created_by).await?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(created)
    }

    /// The compare-and-set of D3 over the three text columns. `settings` is **not** in the `SET`
    /// list: the key-level merge of [`set_setting`](WriteStore::set_setting) is that column's one
    /// writer, so an edit of the name cannot silently drop MOD-4's or MOD-12's keys (D8).
    /// `secret` writes both secret columns in this statement (MOD-10 M4 D7).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] on a `slug`
    /// collision.
    async fn update_project(
        &self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
    ) -> Result<CasOutcome<Project>> {
        let (secret_set, secret_provider, secret_scope) = match &patch.secret {
            None => (false, None, None),
            Some(None) => (true, None, None),
            Some(Some(scope)) => (
                true,
                Some(htui_core::secret::INFISICAL),
                Some(scope.to_column()),
            ),
        };
        let updated = sqlx::query_as!(
            Project,
            r#"
            UPDATE project SET
                slug            = COALESCE($3, slug),
                name            = COALESCE($4, name),
                description     = COALESCE($5, description),
                secret_provider = CASE WHEN $6 THEN $7 ELSE secret_provider END,
                secret_scope    = CASE WHEN $6 THEN $8 ELSE secret_scope END
             WHERE id = $1 AND updated_at = $2
            RETURNING id              AS "id: ProjectId",
                      slug,
                      name,
                      description,
                      secret_provider,
                      secret_scope,
                      settings,
                      created_by      AS "created_by: htui_core::model::UserId",
                      created_at,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.slug,
            patch.name,
            patch.description,
            secret_set,
            secret_provider,
            secret_scope,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        match updated {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => cas_miss(self.project(id).await?, "project", id),
        }
    }

    // repo and repo box paths

    /// `is_primary: true` is honoured rather than refused: the project's current primary is cleared
    /// in the **same transaction**, so `uq_repo_primary` (`0001_init.sql:196`) is never violated,
    /// not even momentarily — the partial unique index is checked per statement and would refuse a
    /// second primary outright (D10).
    ///
    /// A failed insert drops `tx` unsent, so the demotion is rolled back with it: a create that
    /// collides on `(project_id, name)` leaves the old primary primary.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when `(project_id, name)` is taken (`23505`) or `project_id`
    /// names no row (`23503`).
    async fn create_repo(&self, new: NewRepo) -> Result<Repo> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        if new.is_primary {
            sqlx::query!(
                "UPDATE repo SET is_primary = false WHERE project_id = $1 AND is_primary",
                new.project_id.as_uuid(),
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        let created = sqlx::query_as!(
            Repo,
            r#"
            INSERT INTO repo (id, project_id, name, remote_url, default_branch, is_primary)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id         AS "id: RepoId",
                      project_id AS "project_id: ProjectId",
                      name,
                      remote_url,
                      default_branch,
                      is_primary,
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.project_id.as_uuid(),
            new.name,
            new.remote_url,
            new.default_branch,
            new.is_primary,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(created)
    }

    /// The compare-and-set of D3, with [`create_repo`](WriteStore::create_repo)'s demotion in front
    /// of it when the patch promotes this row.
    ///
    /// The demotion runs **before** the row update because the unique index would refuse two
    /// primaries within one statement; it is rolled back with the transaction when the token turns
    /// out to be spent, so a stale promotion demotes nobody. `remote_url` needs two parameters for
    /// the reason [`update_item`](PgStore::update_item)'s `step_graph_id` does: it is a
    /// double-[`Option`] and `None` ("leave it") and `Some(None)` ("clear it") would both arrive as
    /// SQL `NULL`.
    ///
    /// The demoted row's own `updated_at` advances, because its column changed and the trigger
    /// fires per row.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] on a `name`
    /// collision.
    async fn update_repo(
        &self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
    ) -> Result<CasOutcome<Repo>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        if patch.is_primary == Some(true) {
            sqlx::query!(
                "UPDATE repo SET is_primary = false \
                  WHERE project_id = (SELECT project_id FROM repo WHERE id = $1) \
                    AND id <> $1 \
                    AND is_primary",
                id.as_uuid(),
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        let updated = sqlx::query_as!(
            Repo,
            r#"
            UPDATE repo SET
                name           = COALESCE($3, name),
                remote_url     = CASE WHEN $4 THEN $5 ELSE remote_url END,
                default_branch = COALESCE($6, default_branch),
                is_primary     = COALESCE($7, is_primary)
             WHERE id = $1 AND updated_at = $2
            RETURNING id         AS "id: RepoId",
                      project_id AS "project_id: ProjectId",
                      name,
                      remote_url,
                      default_branch,
                      is_primary,
                      created_at,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.name,
            patch.remote_url.is_some(),
            patch.remote_url.flatten(),
            patch.default_branch,
            patch.is_primary,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        match updated {
            Some(row) => {
                tx.commit().await.map_err(map_sqlx)?;
                Ok(CasOutcome::Applied(row))
            }
            None => {
                // The demotion, if there was one, goes back with the transaction: a spent token
                // must leave the project's primary exactly as it found it.
                tx.rollback().await.map_err(map_sqlx)?;
                cas_miss(self.repo_row(id).await?, "repo", id)
            }
        }
    }

    /// A project's repos ordered by `name` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>> {
        self.repo_rows(project).await
    }

    /// One row per `(repo_id, box_id)`, replaced in place (`R-BOX-4`), as
    /// [`upsert_workspace_box_path`](WriteStore::upsert_workspace_box_path).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when either id names no row (`23503`).
    async fn upsert_repo_box_path(&self, path: &RepoBoxPath) -> Result<()> {
        sqlx::query!(
            "INSERT INTO repo_box_path (repo_id, box_id, local_path) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (repo_id, box_id) DO UPDATE SET local_path = EXCLUDED.local_path",
            path.repo_id.as_uuid(),
            path.box_id.as_uuid(),
            path.local_path,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// One row per `(repo_id, box_id)`, inserted only where none exists (MOD-7 milestone 4, D104).
    /// The conflict clause decides it atomically, so a concurrent manual upsert either lands first
    /// (this answers `false`) or replaces what this wrote.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when either id names no row (`23503`). A row that already holds
    /// the pair implies both ids exist, so a conflict never hides a foreign-key refusal.
    async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool> {
        let done = sqlx::query!(
            "INSERT INTO repo_box_path (repo_id, box_id, local_path) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (repo_id, box_id) DO NOTHING",
            path.repo_id.as_uuid(),
            path.box_id.as_uuid(),
            path.local_path,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(done.rows_affected() == 1)
    }

    /// Every box's checkout path for a repo, ordered by `box_id` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn repo_box_paths(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>> {
        self.repo_box_path_rows(repo).await
    }

    // item_kind

    /// The prefix rule in Rust, the graph rule folded into the statement, and the two uniqueness
    /// rules left to the database (D11).
    ///
    /// [`ItemKind::prefix_is_valid`] runs first so the refusal is the sentence `MemStore` uses
    /// rather than `23514` and a constraint name; the `WHERE EXISTS` is the shape
    /// [`mint_item`](PgStore::mint_item)'s kind guard has, because `item_kind.default_graph_id`
    /// references `step_graph(id)` alone (`0001_init.sql:288`) and nothing below the seam checks
    /// that the graph is the kind's own project's. Zero rows out is therefore exactly "the guard
    /// fired", and one follow-up read names which of its two halves.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a bad prefix, an unknown project, a graph from another
    /// project, or a taken `(project_id, prefix)` / `(project_id, name)` (`23505`).
    async fn create_item_kind(&self, new: NewItemKind) -> Result<ItemKind> {
        if !ItemKind::prefix_is_valid(&new.prefix) {
            return Err(StoreError::Constraint(invalid_prefix(&new.prefix)));
        }

        let created = sqlx::query_as!(
            ItemKind,
            r#"
            INSERT INTO item_kind (id, project_id, prefix, name, description, default_graph_id,
                                   position)
            SELECT $1, $2, $3, $4, $5, $6, $7
             WHERE EXISTS (SELECT 1 FROM step_graph g WHERE g.id = $6 AND g.project_id = $2)
            RETURNING id               AS "id: ItemKindId",
                      project_id       AS "project_id: ProjectId",
                      prefix,
                      name,
                      description,
                      default_graph_id AS "default_graph_id: StepGraphId",
                      position,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.project_id.as_uuid(),
            new.prefix,
            new.name,
            new.description,
            new.default_graph_id.as_uuid(),
            new.position,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        match created {
            Some(row) => Ok(row),
            None => Err(self
                .kind_guard_refusal(new.project_id, new.default_graph_id)
                .await),
        }
    }

    /// The compare-and-set of D3 with [`create_item_kind`](WriteStore::create_item_kind)'s three
    /// rules re-applied to the patched values.
    ///
    /// Renaming the prefix touches no `item` and no `item_key_counter`: `item.key` is generated
    /// from the columns copied at mint time (`0001_init.sql:317`) and the counter is keyed by
    /// `(project, prefix)`, so old keys keep their text and the next mint under the kind starts the
    /// new prefix at 1 (PRD D12).
    ///
    /// Zero rows means one of three things, split by one follow-up read the way
    /// [`update_item`](PgStore::update_item) splits its own: no such row, a spent token, or the
    /// graph guard — which can only have fired if the token still matches.
    ///
    /// The prefix rule is checked **behind** the compare-and-set, which is `MemStore`'s order and
    /// the order a `(project, prefix)` collision already has here: a spent token answers `Stale`
    /// whether or not the input was also bad, so the two stores cannot disagree about which refusal
    /// a caller sees (review M2). The bad-input path is the only one that pays for the extra read,
    /// because the statement below cannot be sent with a prefix the column's `CHECK` would refuse
    /// with its own wording.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] as for create.
    async fn update_item_kind(
        &self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    ) -> Result<CasOutcome<ItemKind>> {
        if let Some(prefix) = &patch.prefix
            && !ItemKind::prefix_is_valid(prefix)
        {
            let Some(current) = self.item_kind(id).await? else {
                return Err(StoreError::NotFound {
                    entity: "item_kind",
                    id: id.to_string(),
                });
            };
            if current.updated_at != expected {
                return Ok(CasOutcome::Stale(current));
            }
            return Err(StoreError::Constraint(invalid_prefix(prefix)));
        }

        let updated = sqlx::query_as!(
            ItemKind,
            r#"
            UPDATE item_kind SET
                prefix           = COALESCE($3, prefix),
                name             = COALESCE($4, name),
                description      = COALESCE($5, description),
                default_graph_id = COALESCE($6, default_graph_id),
                position         = COALESCE($7, position)
             WHERE id = $1
               AND updated_at = $2
               AND EXISTS (SELECT 1 FROM step_graph g
                            WHERE g.id = COALESCE($6, item_kind.default_graph_id)
                              AND g.project_id = item_kind.project_id)
            RETURNING id               AS "id: ItemKindId",
                      project_id       AS "project_id: ProjectId",
                      prefix,
                      name,
                      description,
                      default_graph_id AS "default_graph_id: StepGraphId",
                      position,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.prefix,
            patch.name,
            patch.description,
            patch.default_graph_id.map(StepGraphId::as_uuid),
            patch.position,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if let Some(row) = updated {
            return Ok(CasOutcome::Applied(row));
        }

        let Some(current) = self.item_kind(id).await? else {
            return Err(StoreError::NotFound {
                entity: "item_kind",
                id: id.to_string(),
            });
        };
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        // The compare-and-set matched, so the only other conjunct of the `WHERE` - the graph guard
        // - is what refused the row, and the project exists because the row does.
        let graph = patch.default_graph_id.unwrap_or(current.default_graph_id);
        Err(StoreError::Constraint(graph_not_in_project(
            graph,
            current.project_id,
        )))
    }

    /// A project's kinds ordered by `position`, then `prefix` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>> {
        self.item_kind_rows(project).await
    }

    /// Locks the kind, then deletes it under a guard the lock makes authoritative (D6).
    ///
    /// The `item.kind_id` foreign key has no cascade (`0001_init.sql:313`) and would refuse the
    /// delete on its own — with a constraint name where PRD D6 asks for "names what holds it", so
    /// the guard has to be the seam's and [`item_kind_is_held`] is the sentence both stores use.
    ///
    /// It used to be a `count(*)` and then a `DELETE`, two autocommit round trips: an item minted
    /// between them was invisible to the count and present by the time the foreign key was checked,
    /// so the caller got a raw `23503` where the sentence was promised (review L2). Folding the
    /// guard into the `DELETE`'s own `WHERE` is most of the answer but not all of it — at
    /// `READ COMMITTED` a statement has **one** snapshot, taken before it blocks, and waiting on a
    /// row lock does not re-evaluate a `NOT EXISTS` the way it re-evaluates a compare-and-set's
    /// `updated_at = $2`, because the racing mint only *locks* the kind row and never updates it.
    /// So the `SELECT ... FOR UPDATE` comes first: `INSERT INTO item` takes `FOR KEY SHARE` on the
    /// kind it names, which that lock conflicts with, so from there no mint under this kind can
    /// commit and one already in flight is waited for and then **seen** by the next statement's
    /// snapshot. Only then is the guarded delete authoritative.
    ///
    /// `item_key_counter` is keyed by prefix, not by kind, and is never touched: a kind that comes
    /// back under the same prefix must not re-mint keys the project has already issued (§4.1).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] while items reference it; [`StoreError::NotFound`] for an
    /// unknown id.
    async fn delete_item_kind(&self, id: ItemKindId) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let Some(prefix) = sqlx::query_scalar!(
            "SELECT prefix FROM item_kind WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        else {
            return Err(StoreError::NotFound {
                entity: "item_kind",
                id: id.to_string(),
            });
        };

        let removed = sqlx::query_scalar!(
            r#"
            DELETE FROM item_kind
             WHERE id = $1 AND NOT EXISTS (SELECT 1 FROM item WHERE kind_id = $1)
            RETURNING prefix
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        if removed.is_none() {
            // The row is there — it was just locked — so zero rows means the guard fired, and the
            // count is read only to put the number in the sentence.
            let items = sqlx::query_scalar!(
                r#"SELECT count(*) AS "items!" FROM item WHERE kind_id = $1"#,
                id.as_uuid(),
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            return Err(StoreError::Constraint(item_kind_is_held(
                &prefix,
                rows(items),
            )));
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(())
    }

    // step_graph and phase

    /// One `INSERT ... RETURNING`; the phases are [`create_phase`](WriteStore::create_phase)'s.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when `(project_id, name)` is taken (`23505`) or `project_id`
    /// names no row (`23503`).
    async fn create_step_graph(&self, new: NewStepGraph) -> Result<StepGraph> {
        sqlx::query_as!(
            StepGraph,
            r#"
            INSERT INTO step_graph (id, project_id, name, description, is_override)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id         AS "id: StepGraphId",
                      project_id AS "project_id: ProjectId",
                      name,
                      description,
                      -- Inserted, not appended: see `step_graph_row` in `pg/read.rs`.
                      is_override,
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.project_id.as_uuid(),
            new.name,
            new.description,
            new.is_override,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The compare-and-set of D3 over `name` and `description`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] on a `name`
    /// collision.
    async fn update_step_graph(
        &self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    ) -> Result<CasOutcome<StepGraph>> {
        let updated = sqlx::query_as!(
            StepGraph,
            r#"
            UPDATE step_graph SET
                name        = COALESCE($3, name),
                description = COALESCE($4, description)
             WHERE id = $1 AND updated_at = $2
            RETURNING id         AS "id: StepGraphId",
                      project_id AS "project_id: ProjectId",
                      name,
                      description,
                      -- Inserted, not appended: see `step_graph_row` in `pg/read.rs`.
                      is_override,
                      created_at,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.name,
            patch.description,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        match updated {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => cas_miss(self.step_graph_row(id).await?, "step_graph", id),
        }
    }

    /// A project's graphs ordered by `name` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn step_graphs(&self, project: ProjectId) -> Result<Vec<StepGraph>> {
        self.step_graph_rows(project).await
    }

    /// The whole row (D10), minus `updated_at`: the column is left out of the insert so it takes
    /// the server's `now()` and the caller's stamp is discarded.
    ///
    /// `judge` and `handoff` are refused before the statement because they are **template** roles
    /// (ANA-5 §4.6) and nothing in the schema says so: a project cannot hold both a `judge` phase
    /// template and a `judge` judge template, since `prompt_template` is
    /// `UNIQUE (project_id, name, version)`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a reserved name, a taken `(graph_id, position)` or
    /// `(graph_id, name)` (`23505`), a `graph_id` that names no row (`23503`), or a `persona_id`
    /// that names no row (`fk_step_graph_phase_persona`'s `23503`, worded
    /// `references_no_row("step_graph_phase.persona_id", id, "persona")`), checked after the
    /// clashes (MOD-26 D5).
    async fn create_phase(&self, phase: &StepGraphPhase) -> Result<StepGraphPhase> {
        if TemplateRole::of_name(&phase.name) != TemplateRole::Phase {
            return Err(StoreError::Constraint(reserved_phase_name(&phase.name)));
        }

        sqlx::query_as!(
            StepGraphPhase,
            r#"
            INSERT INTO step_graph_phase (id, graph_id, position, name, fan_out, gate, gate_hard,
                                          retry_limit, input_kinds, output_kind, isolation,
                                          command_queue, verify_command, template_name,
                                          template_version, token_budget, persona_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
            RETURNING id               AS "id: PhaseId",
                      graph_id         AS "graph_id: StepGraphId",
                      position,
                      name,
                      fan_out,
                      gate             AS "gate: htui_core::model::Gate",
                      gate_hard,
                      retry_limit,
                      input_kinds,
                      output_kind,
                      isolation        AS "isolation: Isolation",
                      command_queue    AS "command_queue: htui_core::model::CommandQueue",
                      verify_command,
                      template_name,
                      template_version,
                      token_budget,
                      persona_id       AS "persona_id: PersonaId",
                      updated_at
            "#,
            phase.id.as_uuid(),
            phase.graph_id.as_uuid(),
            phase.position,
            phase.name,
            phase.fan_out,
            phase.gate.as_str(),
            phase.gate_hard,
            phase.retry_limit,
            &phase.input_kinds[..],
            phase.output_kind,
            phase.isolation.map(Isolation::as_str),
            phase.command_queue.as_str(),
            phase.verify_command.as_deref(),
            phase.template_name,
            phase.template_version,
            phase.token_budget,
            phase.persona_id.map(PersonaId::as_uuid),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| phase_persona_refused(err, phase.persona_id))
    }

    /// One `UNNEST` insert (MOD-37 R-6), so the batch lands whole or not at all. A row naming
    /// another phase is refused before the statement; everything else is the schema's.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a row whose `phase_id` is not `phase`, a `phase` or an
    /// `agent_id` that names no row (`23503`), or a taken `(phase_id, position)`, stored or
    /// within the batch (`23505`).
    async fn create_phase_agents(&self, phase: PhaseId, agents: &[PhaseAgent]) -> Result<()> {
        if agents.is_empty() {
            return Ok(());
        }
        if let Some(row) = agents.iter().find(|row| row.phase_id != phase) {
            return Err(StoreError::Constraint(row_names_another_phase(
                "phase_agent",
                row.phase_id,
                phase,
            )));
        }
        let positions: Vec<i32> = agents.iter().map(|row| row.position).collect();
        let agent_ids: Vec<Uuid> = agents.iter().map(|row| row.agent_id.as_uuid()).collect();
        let models: Vec<String> = agents.iter().map(|row| row.model.clone()).collect();
        sqlx::query!(
            r#"
            INSERT INTO phase_agent (phase_id, position, agent_id, model)
            SELECT $1, t.position, t.agent_id, t.model
              FROM UNNEST($2::int4[], $3::uuid[], $4::text[]) AS t(position, agent_id, model)
            "#,
            phase.as_uuid(),
            &positions,
            &agent_ids,
            &models,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// The compare-and-set of D3 over [`PhasePatch`]'s six columns.
    ///
    /// `token_budget` is not among them and cannot be: the `Phase` rung of
    /// [`set_setting`](WriteStore::set_setting) is that column's one writer, so the value the
    /// resolver reads has one editor rather than two (D8).
    ///
    /// The reserved-name rule is checked **behind** the compare-and-set, for the reason and in the
    /// shape [`update_item_kind`](WriteStore::update_item_kind) gives (review M2).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] for a reserved name,
    /// a `(graph_id, position)` / `(graph_id, name)` collision (`23505`), or a `persona_id` that
    /// names no row (`fk_step_graph_phase_persona`'s `23503`, worded
    /// `references_no_row("step_graph_phase.persona_id", id, "persona")`), checked after the
    /// clashes (MOD-26 D5).
    async fn update_phase(
        &self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    ) -> Result<CasOutcome<StepGraphPhase>> {
        if let Some(name) = &patch.name
            && TemplateRole::of_name(name) != TemplateRole::Phase
        {
            let Some(current) = self.phase_row(id).await? else {
                return Err(StoreError::NotFound {
                    entity: "step_graph_phase",
                    id: id.to_string(),
                });
            };
            if current.updated_at != expected {
                return Ok(CasOutcome::Stale(current));
            }
            return Err(StoreError::Constraint(reserved_phase_name(name)));
        }

        // MOD-26 D5: `COALESCE` cannot clear, so the binding is a flag and a value; read before
        // the macro, which moves the patch's other fields.
        let persona = patch.persona;
        let updated = sqlx::query_as!(
            StepGraphPhase,
            r#"
            UPDATE step_graph_phase SET
                name          = COALESCE($3, name),
                position      = COALESCE($4, position),
                template_name = COALESCE($5, template_name),
                gate_hard     = COALESCE($6, gate_hard),
                input_kinds   = COALESCE($7, input_kinds),
                persona_id    = CASE WHEN $8::bool THEN $9::uuid ELSE persona_id END
             WHERE id = $1 AND updated_at = $2
            RETURNING id               AS "id: PhaseId",
                      graph_id         AS "graph_id: StepGraphId",
                      position,
                      name,
                      fan_out,
                      gate             AS "gate: htui_core::model::Gate",
                      gate_hard,
                      retry_limit,
                      input_kinds,
                      output_kind,
                      isolation        AS "isolation: Isolation",
                      command_queue    AS "command_queue: htui_core::model::CommandQueue",
                      verify_command,
                      template_name,
                      template_version,
                      token_budget,
                      persona_id       AS "persona_id: PersonaId",
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.name,
            patch.position,
            patch.template_name,
            patch.gate_hard,
            patch.input_kinds.as_deref(),
            persona.is_some(),
            persona.flatten().map(PersonaId::as_uuid),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| phase_persona_refused(err, persona.flatten()))?;

        match updated {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => cas_miss(self.phase_row(id).await?, "step_graph_phase", id),
        }
    }

    /// A graph's phases ordered by `position`.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn phases(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>> {
        self.phase_rows(graph).await
    }

    // prompt_template (MOD-9 milestone 1)

    /// D3 amended by blueprint D16: one `INSERT … SELECT … WHERE head IS NOT DISTINCT FROM $6 ON
    /// CONFLICT DO NOTHING`. Two saves at one head: the second blocks on the unique index and then
    /// inserts nothing (probed on 16.13). Zero rows is split by one head read, `cas_miss`'s
    /// shape. Bad input pays the head read first so a spent token still answers `Stale` (D18,
    /// review M2). A project or `created_by` that names no row is the FK's `23503`, which
    /// [`map_sqlx`] turns into [`StoreError::Constraint`].
    async fn append_prompt_template(
        &self,
        new: NewPromptTemplate,
        expected: Option<i32>,
    ) -> Result<CasOutcome<PromptTemplate>> {
        let key = prompt_template_key(new.project_id, &new.name);
        if let Some(refusal) = prompt_template_refusal(&new.name, &new.body) {
            // No row can be named with a NUL, and binding one is `22021`, not an empty read.
            let head = if new.name.contains('\0') {
                None
            } else {
                self.prompt_template(new.project_id, &new.name, None)
                    .await?
            };
            return match (head, expected) {
                (Some(head), _) if Some(head.version) != expected => Ok(CasOutcome::Stale(head)),
                (None, Some(_)) => Err(StoreError::NotFound {
                    entity: "prompt_template",
                    id: key,
                }),
                _ => Err(StoreError::Constraint(refusal)),
            };
        }

        let inserted = sqlx::query_as!(
            PromptTemplate,
            r#"
            INSERT INTO prompt_template (id, project_id, name, version, body, created_by)
            SELECT $1, $2, $3, COALESCE($6::int, 0) + 1, $4, $5
             WHERE (SELECT max(version) FROM prompt_template
                     WHERE project_id = $2 AND name = $3) IS NOT DISTINCT FROM $6::int
            ON CONFLICT (project_id, name, version) DO NOTHING
            RETURNING id         AS "id: PromptTemplateId",
                      project_id AS "project_id: ProjectId",
                      name,
                      version,
                      body,
                      created_by AS "created_by: UserId",
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.project_id.as_uuid(),
            new.name,
            new.body,
            new.created_by.as_uuid(),
            expected,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if let Some(row) = inserted {
            return Ok(CasOutcome::Applied(row));
        }
        cas_miss(
            self.prompt_template(new.project_id, &new.name, None)
                .await?,
            "prompt_template",
            key,
        )
    }

    // skill, skill_version, skill_binding (MOD-9 milestone 3, plan D75-D79, blueprint D92)

    /// Every skill, ordered by `name` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn skills(&self) -> Result<Vec<Skill>> {
        self.skill_rows().await
    }

    /// One skill's versions, ascending.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn skill_versions(&self, skill: SkillId) -> Result<Vec<SkillVersion>> {
        self.skill_version_rows(skill).await
    }

    /// The global rows, or one project's project and phase rows.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn skill_bindings(&self, project: Option<ProjectId>) -> Result<Vec<SkillBinding>> {
        self.skill_binding_rows(project).await
    }

    /// D92: one explicit transaction of two `INSERT`s, so no skill exists without its version 1.
    /// `MemStore`'s order: the input rules first ([`new_skill_refusal`]); then an unknown
    /// `created_by`, read before the insert because the foreign key is checked only after the
    /// unique indexes (it would lose to a taken name); then a taken id (`skill_pkey`) and a taken
    /// name (`skill_name_key`), both `23505`s worded as `MemStore` words them. The foreign key
    /// stays the backstop. A failed statement drops the transaction, which rolls back.
    async fn create_skill(&self, new: NewSkill) -> Result<(Skill, SkillVersion)> {
        if let Some(refusal) = new_skill_refusal(&new.name, &new.description, &new.body) {
            return Err(StoreError::Constraint(refusal));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let known = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM app_user WHERE id = $1) AS "known!""#,
            new.created_by.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if !known {
            return Err(StoreError::Constraint(unknown_author(
                "skill.created_by",
                new.created_by,
            )));
        }
        let skill = sqlx::query_as!(
            Skill,
            r#"
            INSERT INTO skill (id, name, description, created_by)
            VALUES ($1, $2, $3, $4)
            RETURNING id         AS "id: SkillId",
                      name,
                      description,
                      created_by AS "created_by: UserId",
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.name,
            new.description,
            new.created_by.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| skill_insert_refused(err, &new))?;
        let version = sqlx::query_as!(
            SkillVersion,
            r#"
            INSERT INTO skill_version (skill_id, version, body, source, created_by)
            VALUES ($1, 1, $2, $3, $4)
            RETURNING skill_id   AS "skill_id: SkillId",
                      version,
                      body,
                      source,
                      created_by AS "created_by: UserId",
                      created_at
            "#,
            new.id.as_uuid(),
            new.body,
            new.source,
            new.created_by.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok((skill, version))
    }

    /// D76: the compare-and-set of `update_workspace` with `COALESCE` patch semantics. Bad input
    /// pays one read so the order is `MemStore`'s (`NotFound`, then `Stale`, then `Constraint`); a
    /// spent token matches no row and never reaches the unique index, so a stale rename into a
    /// taken name is `Stale`.
    async fn update_skill(
        &self,
        id: SkillId,
        expected: DateTime<Utc>,
        patch: SkillPatch,
    ) -> Result<CasOutcome<Skill>> {
        if let Some(refusal) = skill_patch_refusal(&patch) {
            return match self.skill_row(id).await? {
                None => Err(StoreError::NotFound {
                    entity: "skill",
                    id: id.to_string(),
                }),
                Some(row) if row.updated_at != expected => Ok(CasOutcome::Stale(row)),
                Some(_) => Err(StoreError::Constraint(refusal)),
            };
        }
        let updated = sqlx::query_as!(
            Skill,
            r#"
            UPDATE skill SET
                name        = COALESCE($3, name),
                description = COALESCE($4, description)
             WHERE id = $1 AND updated_at = $2
            RETURNING id         AS "id: SkillId",
                      name,
                      description,
                      created_by AS "created_by: UserId",
                      created_at,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.name.as_deref(),
            patch.description.as_deref(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| match &patch.name {
            Some(name) => skill_name_taken(err, name),
            None => map_sqlx(err),
        })?;
        match updated {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => cas_miss(self.skill_row(id).await?, "skill", id),
        }
    }

    /// D77, D89: one `INSERT … SELECT … WHERE the head is $expected ON CONFLICT DO NOTHING`, the
    /// `append_prompt_template` shape. Two appends at one head: the second blocks on the primary
    /// key, then inserts nothing (probed, blueprint §0.3). Zero rows, and bad input before any
    /// write, are classified by `skill_version_miss`'s reads in D89's order. An unknown
    /// `created_by` is the FK's `23503`, which [`map_sqlx`] turns into `Constraint`.
    async fn add_skill_version(
        &self,
        skill: SkillId,
        expected: i32,
        new: NewSkillVersion,
    ) -> Result<CasOutcome<SkillVersion>> {
        if let Some(refusal) = skill_body_refusal(&new.body) {
            return skill_version_miss(self, skill, expected, Some(refusal)).await;
        }
        let inserted = sqlx::query_as!(
            SkillVersion,
            r#"
            INSERT INTO skill_version (skill_id, version, body, source, created_by)
            SELECT $1, $2::int + 1, $3, $4, $5
             WHERE EXISTS (SELECT 1 FROM skill WHERE id = $1)
               AND (SELECT COALESCE(max(version), 0) FROM skill_version WHERE skill_id = $1)
                   = $2::int
            ON CONFLICT (skill_id, version) DO NOTHING
            RETURNING skill_id   AS "skill_id: SkillId",
                      version,
                      body,
                      source,
                      created_by AS "created_by: UserId",
                      created_at
            "#,
            skill.as_uuid(),
            expected,
            new.body,
            new.source,
            new.created_by.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        match inserted {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => skill_version_miss(self, skill, expected, None).await,
        }
    }

    /// D90, D92: the row at `key` and the facts [`check_attachment`] needs are read first, then
    /// one statement writes: an attach under `None` is `INSERT … ON CONFLICT (skill_id,
    /// project_id, phase_id) DO NOTHING` (it infers the `NULLS NOT DISTINCT` key, probed), a change
    /// is `UPDATE … WHERE id AND updated_at`, a detach `DELETE … WHERE id AND updated_at`. The
    /// reads are advisory: the write's own `WHERE` or `ON CONFLICT` decides, and zero rows re-read
    /// the key for `Stale`, as `update_workspace`. A project or phase deleted between the reads
    /// and the write is the FK's `23503`, a `Constraint`.
    async fn set_skill_binding(
        &self,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
    ) -> Result<CasOutcome<Option<SkillBinding>>> {
        let current = self.skill_binding_row(key).await?;
        if current.as_ref().map(|row| row.updated_at) != expected {
            return Ok(CasOutcome::Stale(current));
        }
        let skill = self
            .skill_row(key.skill)
            .await?
            .ok_or_else(|| StoreError::NotFound {
                entity: "skill",
                id: key.skill.to_string(),
            })?;
        let project_slug = match key.project {
            Some(id) => {
                self.project(id)
                    .await?
                    .ok_or_else(|| StoreError::NotFound {
                        entity: "project",
                        id: id.to_string(),
                    })?
                    .slug
            }
            None => String::new(),
        };
        let phase_project = match key.phase {
            Some(id) => {
                let phase = self
                    .phase_row(id)
                    .await?
                    .ok_or_else(|| StoreError::NotFound {
                        entity: "step_graph_phase",
                        id: id.to_string(),
                    })?;
                self.step_graph_row(phase.graph_id)
                    .await?
                    .map(|graph| graph.project_id)
            }
            None => None,
        };

        let attachment = match change {
            BindingChange::Detach => {
                let Some(row) = current else {
                    return Ok(CasOutcome::Applied(None));
                };
                let deleted = sqlx::query!(
                    "DELETE FROM skill_binding WHERE id = $1 AND updated_at = $2",
                    row.id.as_uuid(),
                    row.updated_at,
                )
                .execute(&self.pool)
                .await
                .map_err(map_sqlx)?
                .rows_affected();
                return if deleted == 0 {
                    Ok(CasOutcome::Stale(self.skill_binding_row(key).await?))
                } else {
                    Ok(CasOutcome::Applied(None))
                };
            }
            BindingChange::Attach(attachment) => attachment,
        };
        let versions: Vec<i32> = self
            .skill_version_rows(key.skill)
            .await?
            .iter()
            .map(|row| row.version)
            .collect();
        let repos: Vec<String> = match key.project {
            Some(id) => self
                .repo_rows(id)
                .await?
                .into_iter()
                .map(|repo| repo.name)
                .collect(),
            None => Vec::new(),
        };
        let stored = check_attachment(
            &BindingFacts {
                key,
                phase_project,
                skill_name: &skill.name,
                versions: &versions,
                repos: &repos,
                project_slug: &project_slug,
            },
            &attachment,
        )
        .map_err(StoreError::Constraint)?;

        let written = match &current {
            None => sqlx::query_as!(
                SkillBinding,
                r#"
                INSERT INTO skill_binding
                       (id, skill_id, project_id, phase_id, pinned_version, position, activation,
                        globs, languages)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT (skill_id, project_id, phase_id) DO NOTHING
                RETURNING id             AS "id: SkillBindingId",
                          skill_id       AS "skill_id: SkillId",
                          project_id     AS "project_id?: ProjectId",
                          phase_id       AS "phase_id: PhaseId",
                          pinned_version,
                          position,
                          activation     AS "activation: Activation",
                          globs,
                          languages,
                          updated_at
                "#,
                SkillBindingId::new().as_uuid(),
                key.skill.as_uuid(),
                key.project.map(ProjectId::as_uuid),
                key.phase.map(PhaseId::as_uuid),
                attachment.pinned_version,
                attachment.position,
                attachment.activation.as_str(),
                &stored.globs[..],
                &stored.languages[..],
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
            Some(row) => sqlx::query_as!(
                SkillBinding,
                r#"
                UPDATE skill_binding SET
                    pinned_version = $3,
                    position       = $4,
                    activation     = $5,
                    globs          = $6,
                    languages      = $7
                 WHERE id = $1 AND updated_at = $2
                RETURNING id             AS "id: SkillBindingId",
                          skill_id       AS "skill_id: SkillId",
                          project_id     AS "project_id?: ProjectId",
                          phase_id       AS "phase_id: PhaseId",
                          pinned_version,
                          position,
                          activation     AS "activation: Activation",
                          globs,
                          languages,
                          updated_at
                "#,
                row.id.as_uuid(),
                row.updated_at,
                attachment.pinned_version,
                attachment.position,
                attachment.activation.as_str(),
                &stored.globs[..],
                &stored.languages[..],
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
        };
        match written {
            Some(row) => Ok(CasOutcome::Applied(Some(row))),
            None => Ok(CasOutcome::Stale(self.skill_binding_row(key).await?)),
        }
    }

    // persona (MOD-26 milestone 1, plan D4)

    /// Every persona in `name` byte order (`COLLATE "C"`).
    async fn personas(&self) -> Result<Vec<Persona>> {
        self.persona_rows().await
    }

    /// D3 before the statement, then one `INSERT … RETURNING`: `persona_pkey` is the id,
    /// `uq_persona_name` the name ([`persona_insert_refused`], `MemStore`'s order). Both stamps
    /// are the columns' `now()` defaults.
    async fn create_persona(&self, new: NewPersona) -> Result<Persona> {
        if let Some(refusal) = new_persona_refusal(&new) {
            return Err(StoreError::Constraint(refusal));
        }
        let tools = persona_json("persona.tools", &new.tools)?;
        let permission = persona_json("persona.permission", &new.permission)?;
        sqlx::query_as!(
            PersonaRow,
            r#"
            INSERT INTO persona (id, name, description, body, tools, permission)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id          AS "id: PersonaId",
                      name,
                      description,
                      body,
                      tools       AS "tools: Json<PersonaTools>",
                      permission  AS "permission: Json<PersonaPermission>",
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.name,
            new.description,
            new.body,
            tools,
            permission,
        )
        .fetch_one(&self.pool)
        .await
        .map(Persona::from)
        .map_err(|err| persona_insert_refused(err, &new))
    }

    /// `update_skill`'s shape: bad input pays one read for `NotFound` → `Stale` → `Constraint`;
    /// a spent token matches no row and never reaches the unique index. `trg_persona_updated_at`
    /// stamps `updated_at`, an all-`None` patch included.
    async fn update_persona(
        &self,
        id: PersonaId,
        expected: DateTime<Utc>,
        patch: PersonaPatch,
    ) -> Result<CasOutcome<Persona>> {
        if let Some(refusal) = persona_patch_refusal(&patch) {
            return match self.persona_row(id).await? {
                None => Err(StoreError::NotFound {
                    entity: "persona",
                    id: id.to_string(),
                }),
                Some(row) if row.updated_at != expected => Ok(CasOutcome::Stale(row)),
                Some(_) => Err(StoreError::Constraint(refusal)),
            };
        }
        let tools = patch
            .tools
            .as_ref()
            .map(|tools| persona_json("persona.tools", tools))
            .transpose()?;
        let permission = patch
            .permission
            .as_ref()
            .map(|permission| persona_json("persona.permission", permission))
            .transpose()?;
        let updated = sqlx::query_as!(
            PersonaRow,
            r#"
            UPDATE persona SET
                name        = COALESCE($3, name),
                description = COALESCE($4, description),
                body        = COALESCE($5, body),
                tools       = COALESCE($6, tools),
                permission  = COALESCE($7, permission)
             WHERE id = $1 AND updated_at = $2
            RETURNING id          AS "id: PersonaId",
                      name,
                      description,
                      body,
                      tools       AS "tools: Json<PersonaTools>",
                      permission  AS "permission: Json<PersonaPermission>",
                      created_at,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
            patch.name.as_deref(),
            patch.description.as_deref(),
            patch.body.as_deref(),
            tools,
            permission,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| match &patch.name {
            Some(name) => persona_name_taken(err, name),
            None => map_sqlx(err),
        })?;
        match updated {
            Some(row) => Ok(CasOutcome::Applied(row.into())),
            None => cas_miss(self.persona_row(id).await?, "persona", id),
        }
    }

    /// Locks the persona, then deletes it under a guard the lock makes authoritative (MOD-26 M2
    /// D14), `delete_item_kind`'s shape (review L2's reasoning, above).
    ///
    /// A bind (`create_phase`/`update_phase` naming the persona) takes `FOR KEY SHARE` on the
    /// persona row through `fk_step_graph_phase_persona`, which `FOR UPDATE` conflicts with: a bind
    /// in flight is waited for and then **seen** by the guarded `DELETE`'s snapshot (zero rows, the
    /// sentence); a bind that arrives after the lock waits behind it and gets `23503`, which
    /// [`phase_persona_refused`] words as `references_no_row`. Without the lock the deleter itself
    /// would see a raw `23503` (probed, plan "Verified claims"). A guard that fired over holders an
    /// unbind removed before they were read runs again, so the sentence never names zero phases.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] with
    /// [`persona_is_bound`]'s sentence while a phase binds it; [`StoreError::Backend`] when
    /// `DELETE_PERSONA_PASSES` passes find the guard fired over no holder they can name.
    async fn delete_persona(&self, id: PersonaId) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let Some(name) = sqlx::query_scalar!(
            "SELECT name FROM persona WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        else {
            return Err(StoreError::NotFound {
                entity: "persona",
                id: id.to_string(),
            });
        };

        // Each pass is two READ COMMITTED statements, two snapshots: an unbind, phase delete or
        // graph delete that commits between the guard and the holders read leaves the guard
        // fired over holders that are gone. Nothing can become a holder while the row is locked
        // (a bind's `FOR KEY SHARE` waits on it), so the holders only shrink and an empty read
        // means the guarded `DELETE` is simply run again.
        //
        // That rests on `fk_step_graph_phase_persona` being NOT DEFERRABLE: a bind checks it at
        // its own statement and so waits on the `FOR UPDATE` lock above, rather than at a commit
        // that could land between two passes. The holders read joins `step_graph` and `project`,
        // which NOT NULL FKs make total today; should a holder ever be one the guard sees and the
        // join cannot name, the passes are capped (R1 L-3) so the loop ends in an error rather
        // than spinning, and the transaction rolls back with nothing deleted.
        for _ in 0..DELETE_PERSONA_PASSES {
            let removed = sqlx::query_scalar!(
                r#"
            DELETE FROM persona
             WHERE id = $1
               AND NOT EXISTS (SELECT 1 FROM step_graph_phase WHERE persona_id = $1)
            RETURNING id
            "#,
                id.as_uuid(),
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if removed.is_some() {
                tx.commit().await.map_err(map_sqlx)?;
                return Ok(());
            }

            // The row is there - it was just locked - so zero rows means the guard fired; the
            // holders are read only to name them in the sentence.
            let holders = sqlx::query!(
                r#"
                SELECT pr.slug AS "project!", g.name AS "graph!", p.name AS "phase!"
                  FROM step_graph_phase p
                  JOIN step_graph g ON g.id = p.graph_id
                  JOIN project pr ON pr.id = g.project_id
                 WHERE p.persona_id = $1
                "#,
                id.as_uuid(),
            )
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .into_iter()
            .map(|row| (row.project, row.graph, row.phase))
            .collect::<Vec<_>>();
            if !holders.is_empty() {
                return Err(StoreError::Constraint(persona_is_bound(&name, &holders)));
            }
        }

        Err(StoreError::Backend(
            "delete_persona: holders kept changing".to_owned(),
        ))
    }

    // settings (D7, D8)

    /// [`validate`] first, then the rung's own statement (D7, D8).
    ///
    /// Validation runs **before** the compare-and-set on purpose, which is also `MemStore`'s order:
    /// a value the reader would clamp is refused whether or not the caller's token was current, so
    /// "your edit was stale" never stands in for "that number does not mean what you think". The
    /// peer for a `not_above` rule is read from the same rung, so the number the rule compares
    /// against is the one the resolver would itself have resolved.
    ///
    /// Three rungs, three statements: `app_setting` takes an `INSERT ... ON CONFLICT DO NOTHING`
    /// when `expected` is `None` — a conflict is `Stale`, never an overwrite — and a plain
    /// compare-and-set `UPDATE` otherwise; `project.settings` takes
    /// `settings || jsonb_build_object(key, value)`, which is the key-level merge that leaves every
    /// other key alone, guarded by `jsonb_typeof(settings) = 'object'` because `||` would turn a
    /// scalar or an array into an array (review R1 L2); `step_graph_phase.token_budget` takes the
    /// `INTEGER` [`validate`] has already narrowed to `i32::MAX` for this rung (flag C).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for every validation refusal, for `expected: None` on
    /// `Project` / `Phase`, and for a current token over a `project.settings` that is not an
    /// object ([`project_settings_not_an_object`], `MemStore`'s sentence); [`StoreError::NotFound`] for an unknown project, phase or — under a
    /// token — `app_setting` row.
    async fn set_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<StoredSetting>> {
        if let Some(refusal) = rung_refusal(key, rung.flag()) {
            return Err(StoreError::Constraint(refusal));
        }
        let peer = match key.spec().not_above {
            Some(other) => self
                .stored_setting(rung, other)
                .await?
                .and_then(|row| row.value),
            None => None,
        };
        validate(key, rung.flag(), &value, peer.as_ref()).map_err(StoreError::Constraint)?;

        match rung {
            SettingRung::App => {
                let name = key.key();
                let landed = match expected {
                    // "I expect no row": the insert after a clear. A conflict means somebody is
                    // there, and that is `Stale` rather than a silent overwrite.
                    None => sqlx::query_scalar!(
                        r#"
                        INSERT INTO app_setting (key, value) VALUES ($1, $2)
                        ON CONFLICT (key) DO NOTHING
                        RETURNING updated_at AS "updated_at!"
                        "#,
                        name,
                        &value,
                    )
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(map_sqlx)?,
                    Some(token) => sqlx::query_scalar!(
                        r#"
                        UPDATE app_setting SET value = $2
                         WHERE key = $1 AND updated_at = $3
                        RETURNING updated_at AS "updated_at!"
                        "#,
                        name,
                        &value,
                        token,
                    )
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(map_sqlx)?,
                };

                match landed {
                    Some(updated_at) => Ok(CasOutcome::Applied(StoredSetting {
                        value: Some(value),
                        updated_at,
                    })),
                    None => cas_miss(self.stored_setting(rung, key).await?, "app_setting", name),
                }
            }
            SettingRung::Project(id) => {
                let token = expected
                    .ok_or_else(|| StoreError::Constraint(expected_on_row(key, "project")))?;
                let name = key
                    .spec()
                    .project_key
                    .expect("the rung check passed, so the spec names a project key");
                let landed = sqlx::query_scalar!(
                    r#"
                    UPDATE project SET settings = settings || jsonb_build_object($2::text, $3::jsonb)
                     WHERE id = $1 AND updated_at = $4
                       AND jsonb_typeof(settings) = 'object'
                    RETURNING updated_at AS "updated_at!"
                    "#,
                    id.as_uuid(),
                    name,
                    &value,
                    token,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;

                match landed {
                    Some(updated_at) => Ok(CasOutcome::Applied(StoredSetting {
                        value: Some(value),
                        updated_at,
                    })),
                    // The token was current, so what missed is the blob guard (review R1 L2).
                    None => match self.stored_setting(rung, key).await? {
                        Some(stored) if stored.updated_at == token => Err(StoreError::Constraint(
                            project_settings_not_an_object(id, key),
                        )),
                        current => cas_miss(current, "project", id),
                    },
                }
            }
            SettingRung::Phase(id) => {
                let token = expected.ok_or_else(|| {
                    StoreError::Constraint(expected_on_row(key, "step_graph_phase"))
                })?;
                // `validate` has already narrowed this rung to `i32::MAX` (flag C), so the `None`
                // arm is unreachable — and it is still an error rather than an `expect`, because
                // "unreachable" here depends on a guard two modules away and a panic is a poor way
                // to find out it moved (review L6).
                let budget = value
                    .as_i64()
                    .and_then(|number| i32::try_from(number).ok())
                    .ok_or_else(|| {
                        StoreError::Constraint(format!(
                            "`{key}` = {value} does not fit `step_graph_phase.token_budget`, \
                             which is INTEGER"
                        ))
                    })?;
                let landed = sqlx::query_scalar!(
                    r#"
                    UPDATE step_graph_phase SET token_budget = $2
                     WHERE id = $1 AND updated_at = $3
                    RETURNING updated_at AS "updated_at!"
                    "#,
                    id.as_uuid(),
                    budget,
                    token,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;

                match landed {
                    Some(updated_at) => Ok(CasOutcome::Applied(StoredSetting {
                        value: Some(value),
                        updated_at,
                    })),
                    None => cas_miss(
                        self.stored_setting(rung, key).await?,
                        "step_graph_phase",
                        id,
                    ),
                }
            }
        }
    }

    /// The `DELETE` / `settings - key` / `NULL` half of D8, under the same compare-and-set.
    ///
    /// On the `App` rung there is no row left afterwards to carry a token, so the **deleted** row's
    /// comes back and the next [`set_setting`](WriteStore::set_setting) passes `expected: None`
    /// (flag D). `DELETE ... RETURNING` reads the stored stamp rather than a trigger's: the
    /// `BEFORE UPDATE` trigger does not fire on a delete.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when the key is not accepted on the rung;
    /// [`StoreError::NotFound`] when the row — or, on `App`, the setting — does not exist.
    async fn clear_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>> {
        if let Some(refusal) = rung_refusal(key, rung.flag()) {
            return Err(StoreError::Constraint(refusal));
        }

        let (landed, entity, id) = match rung {
            SettingRung::App => {
                let name = key.key();
                let landed = sqlx::query_scalar!(
                    r#"
                    DELETE FROM app_setting WHERE key = $1 AND updated_at = $2
                    RETURNING updated_at AS "updated_at!"
                    "#,
                    name,
                    expected,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                (landed, "app_setting", name.to_owned())
            }
            SettingRung::Project(id) => {
                let name = key
                    .spec()
                    .project_key
                    .expect("the rung check passed, so the spec names a project key");
                let landed = sqlx::query_scalar!(
                    r#"
                    UPDATE project SET settings = settings - $2::text
                     WHERE id = $1 AND updated_at = $3
                       AND jsonb_typeof(settings) = 'object'
                    RETURNING updated_at AS "updated_at!"
                    "#,
                    id.as_uuid(),
                    name,
                    expected,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                if landed.is_none() {
                    // Stale first, then refuse, as `MemStore` orders it: a current token on a
                    // miss means the blob guard fired.
                    return match self.stored_setting(rung, key).await? {
                        Some(stored) if stored.updated_at == expected => Err(
                            StoreError::Constraint(project_settings_not_an_object(id, key)),
                        ),
                        current => cas_miss(current, "project", id),
                    };
                }
                (landed, "project", id.to_string())
            }
            SettingRung::Phase(id) => {
                let landed = sqlx::query_scalar!(
                    r#"
                    UPDATE step_graph_phase SET token_budget = NULL
                     WHERE id = $1 AND updated_at = $2
                    RETURNING updated_at AS "updated_at!"
                    "#,
                    id.as_uuid(),
                    expected,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                (landed, "step_graph_phase", id.to_string())
            }
        };

        match landed {
            Some(updated_at) => Ok(CasOutcome::Applied(StoredSetting {
                value: None,
                updated_at,
            })),
            None => cas_miss(self.stored_setting(rung, key).await?, entity, id),
        }
    }

    /// One setting on one rung with its CAS token; the statement is `PgStore::stored_setting` in
    /// `pg/read.rs`, named rather than linked because it is `pub(crate)` and this doc comment is
    /// public (`rustdoc::private_intra_doc_links`).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when the key is not accepted on the rung.
    async fn setting(&self, rung: SettingRung, key: SettingKey) -> Result<Option<StoredSetting>> {
        if let Some(refusal) = rung_refusal(key, rung.flag()) {
            return Err(StoreError::Constraint(refusal));
        }
        self.stored_setting(rung, key).await
    }

    // queue settings (MOD-12 M2 D8, D9)

    /// One queue key with its token: `App` with `stored_setting`'s `App` literal, `Project` with
    /// its project literal, `Box` through [`PgStore::box_row`] filtered by this user. No new
    /// offline entry.
    async fn queue_setting(
        &self,
        target: QueueTarget,
        key: QueueSetting,
    ) -> Result<Option<QueueStored>> {
        if let Some(refusal) = queue_target_refusal(key, target) {
            return Err(StoreError::Constraint(refusal));
        }
        self.stored_queue_setting(target, key).await
    }

    /// One queue key's compare-and-set (MOD-12 M2 D9). The `App` and `Project` statements are
    /// `set_setting`'s byte for byte, indentation included, so they reuse its offline entries
    /// (MOD-51 F-9: the cache keys a statement by the SHA-256 of its text): they look mis-indented
    /// here on purpose. The `Box` statement is `edit_box`'s guard over one merged key.
    async fn set_queue_setting(
        &self,
        target: QueueTarget,
        key: QueueSetting,
        value: Value,
        expected: QueueToken,
    ) -> Result<CasOutcome<QueueStored>> {
        if let Some(refusal) = queue_target_refusal(key, target) {
            return Err(StoreError::Constraint(refusal));
        }
        if let Some(refusal) = queue_token_refusal(key, target, expected, false) {
            return Err(StoreError::Constraint(refusal));
        }
        key.validate(&value).map_err(StoreError::Constraint)?;
        let name = key.as_str();
        match (target, expected) {
            (QueueTarget::App, QueueToken::Stamp(want)) => {
                let landed = match want {
                    // "I expect no row": a conflict means somebody is there, and that is `Stale`.
                    None => sqlx::query_scalar!(
                        r#"
                        INSERT INTO app_setting (key, value) VALUES ($1, $2)
                        ON CONFLICT (key) DO NOTHING
                        RETURNING updated_at AS "updated_at!"
                        "#,
                        name,
                        &value,
                    )
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(map_sqlx)?,
                    Some(token) => sqlx::query_scalar!(
                        r#"
                        UPDATE app_setting SET value = $2
                         WHERE key = $1 AND updated_at = $3
                        RETURNING updated_at AS "updated_at!"
                        "#,
                        name,
                        &value,
                        token,
                    )
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(map_sqlx)?,
                };
                match landed {
                    Some(updated_at) => Ok(CasOutcome::Applied(QueueStored {
                        value: Some(value),
                        token: QueueToken::Stamp(Some(updated_at)),
                    })),
                    None => cas_miss(
                        self.stored_queue_setting(target, key).await?,
                        "app_setting",
                        name,
                    ),
                }
            }
            (QueueTarget::Project(id), QueueToken::Stamp(Some(token))) => {
                let landed = sqlx::query_scalar!(
                    r#"
                    UPDATE project SET settings = settings || jsonb_build_object($2::text, $3::jsonb)
                     WHERE id = $1 AND updated_at = $4
                       AND jsonb_typeof(settings) = 'object'
                    RETURNING updated_at AS "updated_at!"
                    "#,
                    id.as_uuid(),
                    name,
                    &value,
                    token,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                match landed {
                    Some(updated_at) => Ok(CasOutcome::Applied(QueueStored {
                        value: Some(value),
                        token: QueueToken::Stamp(Some(updated_at)),
                    })),
                    // The token was current, so what missed is the blob guard (review R1 L2).
                    None => match self.stored_queue_setting(target, key).await? {
                        Some(stored) if stored.token == expected => Err(StoreError::Constraint(
                            project_settings_not_an_object(id, key),
                        )),
                        current => cas_miss(current, "project", id),
                    },
                }
            }
            (QueueTarget::Box(id), QueueToken::EditVersion(want)) => {
                let me = self.this_user();
                let written = sqlx::query!(
                    r#"
                    UPDATE box
                       SET settings     = settings || jsonb_build_object($3::text, $4::jsonb),
                           edit_version = edit_version + 1
                     WHERE id = $1 AND user_id = $5 AND edit_version = $2
                       AND jsonb_typeof(settings) = 'object'
                    RETURNING settings, edit_version
                    "#,
                    id.as_uuid(),
                    want,
                    name,
                    &value,
                    me.as_uuid(),
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                match written {
                    Some(row) => Ok(CasOutcome::Applied(QueueStored {
                        value: row.settings.get(name).cloned(),
                        token: QueueToken::EditVersion(row.edit_version),
                    })),
                    None => self.box_queue_miss(id, want, key).await,
                }
            }
            // `queue_token_refusal` has refused every other pair; one it let through by mistake is a
            // refusal here too, not a panic in the store (review R1).
            _ => Err(StoreError::Constraint(format!(
                "`{key}` on {target:?} does not take {expected:?}"
            ))),
        }
    }

    /// One queue key's removal (MOD-12 M2 D9): `clear_setting`'s `App` and `Project` statements
    /// byte for byte (mis-indented on purpose, as above), and the `Box` guard of
    /// [`set_queue_setting`](WriteStore::set_queue_setting) over `settings - key`.
    async fn clear_queue_setting(
        &self,
        target: QueueTarget,
        key: QueueSetting,
        expected: QueueToken,
    ) -> Result<CasOutcome<QueueStored>> {
        if let Some(refusal) = queue_target_refusal(key, target) {
            return Err(StoreError::Constraint(refusal));
        }
        if let Some(refusal) = queue_token_refusal(key, target, expected, true) {
            return Err(StoreError::Constraint(refusal));
        }
        let name = key.as_str();
        match (target, expected) {
            (QueueTarget::App, QueueToken::Stamp(Some(token))) => {
                let landed = sqlx::query_scalar!(
                    r#"
                    DELETE FROM app_setting WHERE key = $1 AND updated_at = $2
                    RETURNING updated_at AS "updated_at!"
                    "#,
                    name,
                    token,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                match landed {
                    // No row is left to carry a token: the next set presents `Stamp(None)`.
                    Some(_) => Ok(CasOutcome::Applied(QueueStored {
                        value: None,
                        token: QueueToken::Stamp(None),
                    })),
                    None => cas_miss(
                        self.stored_queue_setting(target, key).await?,
                        "app_setting",
                        name,
                    ),
                }
            }
            (QueueTarget::Project(id), QueueToken::Stamp(Some(token))) => {
                let landed = sqlx::query_scalar!(
                    r#"
                    UPDATE project SET settings = settings - $2::text
                     WHERE id = $1 AND updated_at = $3
                       AND jsonb_typeof(settings) = 'object'
                    RETURNING updated_at AS "updated_at!"
                    "#,
                    id.as_uuid(),
                    name,
                    token,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                match landed {
                    Some(updated_at) => Ok(CasOutcome::Applied(QueueStored {
                        value: None,
                        token: QueueToken::Stamp(Some(updated_at)),
                    })),
                    // The token was current, so what missed is the blob guard.
                    None => match self.stored_queue_setting(target, key).await? {
                        Some(stored) if stored.token == expected => Err(StoreError::Constraint(
                            project_settings_not_an_object(id, key),
                        )),
                        current => cas_miss(current, "project", id),
                    },
                }
            }
            (QueueTarget::Box(id), QueueToken::EditVersion(want)) => {
                let me = self.this_user();
                let written = sqlx::query!(
                    r#"
                    UPDATE box
                       SET settings     = settings - $3::text,
                           edit_version = edit_version + 1
                     WHERE id = $1 AND user_id = $4 AND edit_version = $2
                       AND jsonb_typeof(settings) = 'object'
                    RETURNING settings, edit_version
                    "#,
                    id.as_uuid(),
                    want,
                    name,
                    me.as_uuid(),
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                match written {
                    Some(row) => Ok(CasOutcome::Applied(QueueStored {
                        value: row.settings.get(name).cloned(),
                        token: QueueToken::EditVersion(row.edit_version),
                    })),
                    None => self.box_queue_miss(id, want, key).await,
                }
            }
            // `queue_token_refusal` has refused every other pair; one it let through by mistake is a
            // refusal here too, not a panic in the store (review R1).
            _ => Err(StoreError::Constraint(format!(
                "`{key}` on {target:?} does not take {expected:?}"
            ))),
        }
    }

    // deletes (D4)

    /// What a delete would remove, counted without removing (PRD D13).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    async fn delete_reach(&self, target: DeleteTarget) -> Result<Option<DeleteReach>> {
        let mut conn = self.pool.acquire().await.map_err(map_sqlx)?;
        match target {
            DeleteTarget::Workspace(id) => workspace_reach(&mut conn, id).await,
            DeleteTarget::Project(id) => project_reach(&mut conn, id).await,
        }
    }

    /// Counts inside the transaction, then lets the cascade do the deleting
    /// (`0001_init.sql:162,174`). Projects survive it.
    ///
    /// Up to `DELETE_ATTEMPTS` tries, because the transaction runs at `REPEATABLE READ` and a link
    /// or a box path committed under it raises `40001` rather than slipping past the count (review
    /// M1). Both are named rather than linked because they are private and this doc comment is
    /// public (`rustdoc::private_intra_doc_links`).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] when every attempt
    /// lost the race, in which case nothing was deleted.
    async fn delete_workspace(&self, id: WorkspaceId) -> Result<DeleteReach> {
        for _ in 0..DELETE_ATTEMPTS {
            if let Some(reach) = self.delete_workspace_once(id).await? {
                return Ok(reach);
            }
        }
        Err(StoreError::Constraint(concurrent_write(
            "workspace",
            id,
            "links or box paths",
        )))
    }

    /// One `DELETE FROM project` and twenty tables of `ON DELETE CASCADE` behind it (PRD D13),
    /// with the counts taken in the same transaction **and the same snapshot** so the report is the
    /// act.
    ///
    /// Up to `DELETE_ATTEMPTS` tries, for the reason
    /// [`delete_workspace`](WriteStore::delete_workspace) has them.
    ///
    /// The mirror rebuild afterwards is the caller's, not the seam's (D5): `PgStore` holds no
    /// `CacheStore`, and milestone 3's store worker is the one handle that holds both.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] when every attempt
    /// lost the race, in which case nothing was deleted.
    async fn delete_project(&self, id: ProjectId) -> Result<DeleteReach> {
        for _ in 0..DELETE_ATTEMPTS {
            if let Some(reach) = self.delete_project_once(id).await? {
                return Ok(reach);
            }
        }
        Err(StoreError::Constraint(concurrent_write(
            "project",
            id,
            "rows the cascade would take",
        )))
    }

    // -------------------------------------------------------------------------------------------
    // ANA-2 §8's eighteen run writers (MOD-4 milestone 1, plan D1).
    //
    // Declared here from the commit that makes the crate compile again (blueprint H-8): the trait
    // has no default bodies, so `PgStore` cannot satisfy `WriteStore` without them, and the SQL of
    // each lands in the blueprint §3.11 commit that owns it — creation, admission, lease and CAS
    // in 4; settle, gates, fan-out, trees and commits in 5; documents, close-out and failure in 6.
    // The conformance cases that call the still-unwritten ones are red until then, which is what
    // `EXPECTED_CASES` still saying 36 records.
    // -------------------------------------------------------------------------------------------

    /// The `run` row and the item's move to `queued`, together or not at all (plan D6, ANA-2 §5.1).
    ///
    /// The item is locked `FOR UPDATE` before the law is consulted, so the status the refusal
    /// names and the status the `UPDATE` matches on are the same one: between them nothing else
    /// can move the row. That lock is also what makes the "impossible" zero-row branch below
    /// impossible rather than merely unlikely.
    ///
    /// Every other rule is the schema's: `project_id`, `target_box_id`, `started_by` and each
    /// `repo_scope` entry are foreign keys, a repeated `id` is the primary key, and a `graph` run
    /// without a snapshot is `ck_run_graph_snapshot` — all `23xxx`, all
    /// [`StoreError::Constraint`] through [`map_sqlx`], which is the same answer `MemStore` spells
    /// out by hand.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "item" }` for an unknown item;
    /// [`StoreError::Constraint`] when the item cannot reach `queued`, when the item is not in
    /// `new.project_id`, when the id is taken, or on any of the keys above. Nothing is written on
    /// any of them. `NotFound { entity: "queue_batch" }` / `Constraint` for an unknown or closed
    /// `new.batch_id` (MOD-12 D7).
    async fn create_run(&self, new: NewRun) -> Result<Run> {
        let snapshot = serde_json::to_value(&new.graph_snapshot).map_err(|error| {
            StoreError::Constraint(format!("run.graph_snapshot does not serialise: {error}"))
        })?;
        let scope: Vec<Uuid> = new
            .repo_scope
            .iter()
            .copied()
            .map(RepoId::as_uuid)
            .collect();

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let item = sqlx::query!(
            r#"SELECT status     AS "status: Status",
                      project_id AS "project_id: ProjectId"
                 FROM item WHERE id = $1 FOR UPDATE"#,
            new.item_id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: new.item_id.to_string(),
        })?;
        let status = item.status;
        legal_move(status, Status::Queued)?;
        // `run.project_id` and `item.project_id` are independent foreign keys, so the schema
        // cannot refuse a run filed under project A for project B's item - and `delete_project`,
        // which counts by `run.project_id`, would then take it with the wrong project. The row is
        // already locked and read, so the comparison costs nothing.
        if item.project_id != new.project_id {
            return Err(StoreError::Constraint(item_not_in_project(
                new.item_id,
                new.project_id,
            )));
        }

        // `run.repo_scope` is a `UUID[]`, and an array element cannot carry a `REFERENCES` clause,
        // so the schema cannot refuse a scope naming a repo that does not exist - this writer has
        // to, because `MemStore` does (`references_no_row("run.repo_scope", ...)`) and because
        // `claim_run`'s overlap test `repo_scope && $2::uuid[]` would otherwise compare against a
        // phantom id for the rest of the run's life. Inside the transaction and before the first
        // write, so a refusal leaves nothing behind.
        let phantom = sqlx::query_scalar!(
            r#"SELECT scoped.id AS "id!: RepoId"
                 FROM unnest($1::uuid[]) AS scoped(id)
                 LEFT JOIN repo ON repo.id = scoped.id
                WHERE repo.id IS NULL
                LIMIT 1"#,
            &scope,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if let Some(missing) = phantom {
            return Err(StoreError::Constraint(references_no_row(
                "run.repo_scope",
                missing,
                "repo",
            )));
        }

        // MOD-12 D7, H-6: a run joins only an open batch. `FOR SHARE` conflicts with
        // `close_batch`'s UPDATE, so a pause and an admission serialise on the batch row and a run
        // never joins a batch that has already closed. The lock order here is item, then batch;
        // `close_batch` locks batch, then run, then item, but only the items of the batch's
        // `queued` runs, which are `queued` themselves - and this item is `open` or `failed`, or
        // `legal_move` above refused it before the batch lock - so the two never wait on one item.
        if let Some(batch) = new.batch_id {
            let open = sqlx::query_scalar!(
                r#"SELECT closed_at IS NULL AS "open!" FROM queue_batch WHERE id = $1 FOR SHARE"#,
                batch.as_uuid(),
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or_else(|| StoreError::NotFound {
                entity: "queue_batch",
                id: batch.to_string(),
            })?;
            if !open {
                return Err(StoreError::Constraint(batch_is_closed(batch)));
            }
        }

        let run = sqlx::query_as!(
            Run,
            r#"
            INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id,
                             executing_box_id, graph_snapshot, started_by, queued_at, repo_scope,
                             batch_id)
            VALUES ($1, $2, $3, 'graph', $4, 'queued', $5, NULL, $6, $7, $8, $9::uuid[], $10)
            RETURNING id               AS "id: RunId",
                      project_id       AS "project_id: ProjectId",
                      item_id          AS "item_id: ItemId",
                      kind             AS "kind: RunKind",
                      mode             AS "mode: RunMode",
                      status           AS "status: RunStatus",
                      target_box_id    AS "target_box_id: BoxId",
                      executing_box_id AS "executing_box_id: BoxId",
                      graph_snapshot,
                      started_by       AS "started_by: UserId",
                      queued_at,
                      started_at,
                      finished_at,
                      failure,
                      repo_scope       AS "repo_scope: Vec<RepoId>",
                      lease_box_id     AS "lease_box_id: BoxId",
                      lease_expires_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.project_id.as_uuid(),
            new.item_id.as_uuid(),
            new.mode.as_str(),
            new.target_box_id.as_uuid(),
            snapshot,
            new.started_by.as_uuid(),
            new.queued_at,
            &scope,
            new.batch_id.map(BatchId::as_uuid),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // `closed_at` follows the *current* status, exactly as `transition` writes it: `queued` is
        // not terminal, so a retry out of `failed` clears the stamp the failure left.
        let moved = sqlx::query!(
            "UPDATE item SET status = 'queued', closed_at = NULL WHERE id = $1 AND status = $2",
            new.item_id.as_uuid(),
            status.as_str(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        if moved != 1 {
            // The item has been locked `FOR UPDATE` since before the law was consulted, so the
            // status this `UPDATE` matches on is the status the refusal above was decided against
            // and no other transaction can have moved it. Reported as a backend fault rather than
            // dressed up as a caller's constraint, which is `update_item`'s precedent: the
            // alternative sentence - a lost race between attempts - is one this writer has no
            // attempts to lose.
            return Err(StoreError::Backend(format!(
                "item `{}` did not move `{status}` -> `queued` under the row lock this \
                 transaction holds: no rule of ANA-2 §4.3 can produce this",
                new.item_id,
            )));
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(run)
    }

    /// ANA-2 §4.7's admission, one transaction with the box row locked for its whole length.
    ///
    /// The lease's expiry is `clock_timestamp()` plus the TTL, in the statement that admits the
    /// run, so no process's clock enters a lease (MOD-40 plan D10); `started_at` and a
    /// `MissingTags` `finished_at` are the caller's `at` (blueprint B25). The product
    /// `$5 * interval '1 microsecond'` keeps the span in the interval's time field, so the sum is
    /// absolute in any session `TimeZone` (P-13).
    ///
    /// `SELECT ... FOR UPDATE` on `box` is the critical section (§4.7, `docs/ANA-2.md:1094`): the
    /// slot count and the overlap check are read-then-write decisions, and without the lock two
    /// claimers can both read "one slot free" and both take it. The Postgres-only pin is
    /// `pg_criteria.rs::admission_is_serialised_by_the_box_row_lock`.
    ///
    /// **Two predicates over two sets**, which §4.7 draws apart on purpose. The slot count is
    /// `status = 'running'` alone — "an `awaiting_approval` run consumes no compute and must not
    /// hold a slot" (`docs/ANA-2.md:1110`) — while the overlap predicate ranges over the
    /// non-terminal set, `awaiting_approval` included, because a parked run still owns its trees
    /// and its unmerged branch (invariant 6). A `queued` run is in neither: it has no
    /// `executing_box_id`.
    ///
    /// The overlap check reads the live rows that share a repo (`repo_scope && $2`) in
    /// `(queued_at, id)` order and decides §4.7's rules L, I, P in Rust with the predicate
    /// `MemStore` shares, each side's scope decoded from its `graph_snapshot` by `scope_of`
    /// (plan D80, D111). An empty `repo_scope` intersects nothing — `'{}' && anything` is false
    /// — so a run that declared no scope is never refused for overlap (hazard H-10).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a TTL outside [`lease_ttl_micros`]'s range, before the
    /// transaction opens. [`StoreError::NotFound`] `{ entity: "run" }` or `{ entity: "box" }`, the
    /// run looked up first. Every refusal that is not an error is an `Ok` [`Claim`] other than
    /// [`Claim::Admitted`], with nothing written, except [`Claim::MissingTags`]: the run is failed
    /// and its item blocked in this same transaction, which is then committed (MOD-7 milestone 3,
    /// D80). The run row and the box row are both locked `FOR UPDATE` before the tag read, so a
    /// concurrent `record_box_probe` or `edit_box` waits for the decision and the check cannot
    /// race a re-probe.
    ///
    /// The item row is **not** locked. A concurrent `update_item` of `required_tags` is ordered
    /// after the claim: the claim decides on the tags it read, the same end state `MemStore`
    /// reaches when the edit lands just after it. No lock, because `create_run` locks the item
    /// and then the box (through `run.target_box_id`'s foreign key); an item lock taken here after
    /// the box lock would take the two in the opposite order and could deadlock.
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
    ) -> Result<Claim> {
        let ttl = lease_ttl_micros(ttl)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let claimed = sqlx::query!(
            r#"
            SELECT status         AS "status: RunStatus",
                   target_box_id  AS "target_box_id: BoxId",
                   repo_scope     AS "repo_scope: Vec<RepoId>",
                   graph_snapshot
              FROM run WHERE id = $1 FOR UPDATE
            "#,
            run.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "run",
            id: run.to_string(),
        })?;

        let settings = sqlx::query_scalar!(
            "SELECT settings FROM box WHERE id = $1 FOR UPDATE",
            box_id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "box",
            id: box_id.to_string(),
        })?;

        if claimed.status != RunStatus::Queued || claimed.target_box_id != box_id {
            return Ok(Claim::NotClaimable);
        }

        // R-ORCH-10 at claim (MOD-7 milestone 3, D80, D81): inside the admission transaction,
        // after claimability and before the slot and the overlap. The shipped read's form
        // (`pg/read.rs`'s `missing_tags`), joined through the run; a chat run joins no item and is
        // never refused.
        let missing = sqlx::query_scalar!(
            r#"
            SELECT DISTINCT t COLLATE "C" AS "tag!"
              FROM run r
              JOIN item i ON i.id = r.item_id
             CROSS JOIN box b, UNNEST(i.required_tags) t
             WHERE r.id = $1 AND b.id = $2
               AND t <> ALL (b.probed_tags || b.declared_tags)
             ORDER BY 1
            "#,
            run.as_uuid(),
            box_id.as_uuid(),
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if !missing.is_empty() {
            sqlx::query!(
                "UPDATE run \
                    SET status      = 'failed', \
                        failure     = $2, \
                        finished_at = COALESCE(finished_at, $3) \
                  WHERE id = $1 AND status = 'queued'",
                run.as_uuid(),
                missing_tags_failure(&missing),
                at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            // A stale item status is not a refusal: zero rows here means someone else already
            // moved the item on (the admitted branch's rule).
            sqlx::query!(
                "UPDATE item SET status = 'blocked' \
                  WHERE id = (SELECT item_id FROM run WHERE id = $1) AND status = 'queued'",
                run.as_uuid(),
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(Claim::MissingTags { missing });
        }

        let limit = match serde_json::from_value::<BoxSettings>(settings)
            .ok()
            .and_then(|settings| settings.max_concurrent_items)
        {
            Some(limit) => limit,
            None => sqlx::query_scalar!(
                "SELECT value FROM app_setting WHERE key = 'max_concurrent_items'"
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(DEFAULT_MAX_CONCURRENT_ITEMS),
        };

        let running = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM run WHERE executing_box_id = $1 AND status = 'running'",
            box_id.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .unwrap_or(0);
        let running = rows(running);
        if running >= u64::from(limit) {
            return Ok(Claim::SlotFull { running, limit });
        }

        let scope: Vec<Uuid> = claimed
            .repo_scope
            .iter()
            .copied()
            .map(RepoId::as_uuid)
            .collect();
        // The live rows sharing a repo with the claim, in `(queued_at, id)` order, read while the
        // box row is still locked; §4.7's rules are decided here in Rust by the one predicate both
        // stores share (plan D80, D111). A scope-less snapshot reads conservatively.
        let live = sqlx::query!(
            r#"
            SELECT id AS "id: RunId", graph_snapshot, repo_scope AS "repo_scope: Vec<RepoId>"
              FROM run
             WHERE executing_box_id = $1 AND status IN ('running','awaiting_approval')
               AND repo_scope && $2::uuid[]
             ORDER BY queued_at, id
            "#,
            box_id.as_uuid(),
            &scope,
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let mine = scope_of(
            claimed.graph_snapshot.as_ref().unwrap_or(&Value::Null),
            &claimed.repo_scope,
        );
        if let Some(verdict) = live.iter().find_map(|row| {
            let theirs = scope_of(
                row.graph_snapshot.as_ref().unwrap_or(&Value::Null),
                &row.repo_scope,
            );
            overlaps(&mine, &theirs).map(|rule| Claim::Overlaps { with: row.id, rule })
        }) {
            return Ok(verdict);
        }

        sqlx::query!(
            "UPDATE run \
                SET status           = 'running', \
                    executing_box_id = $2, \
                    started_at       = COALESCE(started_at, $4), \
                    lease_box_id     = $2, \
                    lease_owner      = $3, \
                    lease_expires_at = clock_timestamp() + $5::bigint * interval '1 microsecond' \
              WHERE id = $1 AND status = 'queued'",
            run.as_uuid(),
            box_id.as_uuid(),
            owner,
            at,
            ttl,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // A stale item status is not a refusal: the run is what is being claimed, and zero rows
        // here means someone else already moved the item on.
        sqlx::query!(
            "UPDATE item SET status = 'in_progress', closed_at = NULL \
              WHERE id = (SELECT item_id FROM run WHERE id = $1) AND status = 'queued'",
            run.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(Claim::Admitted)
    }

    /// ANA-2 §4.9's heartbeat: a compare-and-set on `lease_owner`, not on the expiry.
    ///
    /// Zero rows is the abandon signal, and it is `Ok(false)` rather than an error — the caller is
    /// meant to stop working, not to crash. `lease_owner` is not a [`Run`] field (blueprint F-S),
    /// so this answer is the only way ownership is observable. The new expiry is
    /// `clock_timestamp()` plus the TTL (MOD-40 plan D10).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a TTL out of range, before anything is read.
    /// [`StoreError::NotFound`] `{ entity: "run" }` when there is no such run at all, told apart
    /// from "not ours" by the one follow-up read [`WriteStore::transition`] has always used.
    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool> {
        let ttl = lease_ttl_micros(ttl)?;
        let moved = sqlx::query!(
            "UPDATE run \
                SET lease_expires_at = clock_timestamp() + $3::bigint * interval '1 microsecond' \
              WHERE id = $1 AND lease_owner = $2",
            run.as_uuid(),
            owner,
            ttl,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }

        let exists = sqlx::query_scalar!("SELECT 1 FROM run WHERE id = $1", run.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_some();

        if exists {
            Ok(false)
        } else {
            Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            })
        }
    }

    /// ANA-2 §4.9's sweep: every `running` **graph** run on the box whose lease is `NULL` or
    /// expired by `clock_timestamp()`, and is not already `owner`'s (plan D88), becomes `owner`'s,
    /// in one statement, its lease `clock_timestamp()` plus the TTL (MOD-40 plan D10).
    ///
    /// A chat run (`kind = 'chat'`, `running` with no lease, `start_chat_run`) is never the
    /// sweep's: the engine cannot recover it, and leasing it fenced the chat's own unleased writes
    /// until `unrecovered` gave it back (MOD-24 D3b).
    ///
    /// The candidates are locked in `(queued_at, id)` order with `FOR UPDATE SKIP LOCKED`
    /// (blueprint A-5): two concurrent sweeps never wait on each other's row locks and cannot
    /// deadlock, and a row one sweep holds is simply not the other's to adopt.
    ///
    /// `RETURNING` cannot carry an `ORDER BY`, so the update is a CTE and the ordering is the
    /// `SELECT` over it — `queued_at`, as the contract says. A box that does not exist matches
    /// nothing and adopts nothing, which is an empty vector rather than a refusal.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a TTL out of range, before anything is read; otherwise the
    /// backend's own failures only.
    async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<Vec<Run>> {
        let ttl = lease_ttl_micros(ttl)?;
        sqlx::query_as!(
            Run,
            r#"
            WITH candidates AS (
                SELECT id
                  FROM run
                 WHERE executing_box_id = $1
                   AND status = 'running'
                   AND kind = 'graph'
                   AND (lease_expires_at IS NULL OR lease_expires_at <= clock_timestamp())
                   AND lease_owner IS DISTINCT FROM $2
                 ORDER BY queued_at, id
                   FOR UPDATE SKIP LOCKED
            ),
            swept AS (
                UPDATE run r
                   SET lease_owner      = $2,
                       lease_box_id     = $1,
                       lease_expires_at = clock_timestamp() + $3::bigint * interval '1 microsecond'
                  FROM candidates c
                 WHERE r.id = c.id
             RETURNING r.*
            )
            SELECT id               AS "id: RunId",
                   project_id       AS "project_id: ProjectId",
                   item_id          AS "item_id: ItemId",
                   kind             AS "kind: RunKind",
                   mode             AS "mode: RunMode",
                   status           AS "status: RunStatus",
                   target_box_id    AS "target_box_id: BoxId",
                   executing_box_id AS "executing_box_id: BoxId",
                   graph_snapshot,
                   started_by       AS "started_by: UserId",
                   queued_at,
                   started_at,
                   finished_at,
                   failure,
                   repo_scope       AS "repo_scope: Vec<RepoId>",
                   lease_box_id     AS "lease_box_id: BoxId",
                   lease_expires_at,
                   updated_at
              FROM swept
             ORDER BY queued_at, id
            "#,
            box_id.as_uuid(),
            owner,
            ttl,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// Plan D87: the lease of a run that is ours or free, in one compare-and-set `UPDATE`.
    ///
    /// Expiry is judged, and the new one stamped, by `clock_timestamp()` (MOD-40 plan D10). Under
    /// READ COMMITTED a take that waits on another's row lock re-evaluates its `WHERE` against the
    /// committed row, and `clock_timestamp()` is volatile, so the re-check reads the clock after
    /// the wait (`pg_criteria.rs::two_takes_of_one_released_lease_admit_one`).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a TTL out of range, before anything is read.
    /// [`StoreError::NotFound`] `{ entity: "run" }` when there is no such run at all, told apart
    /// from "not takeable" by the follow-up read [`WriteStore::refresh_lease`] uses.
    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<bool> {
        let ttl = lease_ttl_micros(ttl)?;
        let moved = sqlx::query!(
            "UPDATE run \
                SET lease_owner      = $3, \
                    lease_box_id     = $2, \
                    lease_expires_at = clock_timestamp() + $4::bigint * interval '1 microsecond' \
              WHERE id = $1 \
                AND status IN ('running','awaiting_approval') \
                AND executing_box_id = $2 \
                AND (lease_owner = $3 OR lease_owner IS NULL \
                     OR lease_expires_at IS NULL OR lease_expires_at <= clock_timestamp())",
            run.as_uuid(),
            box_id.as_uuid(),
            owner,
            ttl,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }

        let exists = sqlx::query_scalar!("SELECT 1 FROM run WHERE id = $1", run.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_some();

        if exists {
            Ok(false)
        } else {
            Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            })
        }
    }

    /// Plan D139: gives a lease back, in one compare-and-set `UPDATE` that clears the owner.
    ///
    /// Clearing `lease_owner` is what lets this process's own sweep adopt the run (plan D88 skips
    /// only a row whose owner is the sweeper). A heartbeat `UPDATE` that commits after this one
    /// filters on `lease_owner = $2` and so matches no row. The expiry is `clock_timestamp()`, so
    /// the lease reads as lapsed at once by every process's comparison (MOD-40 plan D10).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run" }` when there is no such run at all, told apart
    /// from "not ours" by the follow-up read [`WriteStore::refresh_lease`] uses.
    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool> {
        let moved = sqlx::query!(
            "UPDATE run SET lease_owner = NULL, lease_expires_at = clock_timestamp() \
              WHERE id = $1 AND lease_owner = $2",
            run.as_uuid(),
            owner,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }

        let exists = sqlx::query_scalar!("SELECT 1 FROM run WHERE id = $1", run.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_some();

        if exists {
            Ok(false)
        } else {
            Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            })
        }
    }

    /// A `run_step` at `pending` with every settle column `NULL`; `fanout_index = -1` is the judge
    /// and is accepted, because no `CHECK` on that column exists or may be added (ANA-2 risk 12).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] on an unknown run or agent (foreign key), a repeated `id`
    /// (primary key) or a repeated `(run_id, position, attempt, fanout_index)` — the table's own
    /// `UNIQUE`, which is why this writer has no guard of its own.
    async fn create_step(&self, new: NewRunStep) -> Result<RunStep> {
        sqlx::query_as!(
            RunStep,
            r#"
            INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name,
                                  agent_id, model, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending')
            RETURNING id                  AS "id: StepId",
                      run_id              AS "run_id: RunId",
                      position,
                      attempt,
                      fanout_index,
                      phase_name,
                      agent_id            AS "agent_id: AgentId",
                      model,
                      status              AS "status: StepStatus",
                      gate_outcome        AS "gate_outcome: GateOutcome",
                      gate_note,
                      selected,
                      exit_code,
                      prompt_digest,
                      trim_record,
                      usage,
                      isolation_path,
                      started_at,
                      finished_at,
                      verify_outcome      AS "verify_outcome: VerifyOutcome",
                      verify_exit_code,
                      promoted_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.run_id.as_uuid(),
            new.position,
            new.attempt,
            new.fanout_index,
            new.phase_name,
            new.agent_id.map(AgentId::as_uuid),
            new.model,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// §4.3's compare-and-set on `run.status`, in [`WriteStore::transition`]'s shape with §4.3's
    /// two stamps: `started_at` on the move to `running`, `finished_at` on a move to a terminal
    /// status, both `COALESCE`d so a second arrival never re-stamps.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run" }`; [`StoreError::Constraint`] for a pair outside
    /// [`RunStatus::can_move_to`], refused before the `UPDATE` (plan D14/D15).
    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        if let Err(refusal) = legal_move(from, to) {
            let exists = sqlx::query_scalar!("SELECT 1 FROM run WHERE id = $1", run.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?
                .is_some();
            return refuse_illegal_move::<RunStatus, _>(exists, run, refusal);
        }

        let moved = sqlx::query!(
            "UPDATE run \
                SET status      = $3, \
                    started_at  = CASE WHEN $3 = 'running' \
                                       THEN COALESCE(started_at, $4) ELSE started_at END, \
                    finished_at = CASE WHEN $3 IN ('done','failed','cancelled') \
                                       THEN COALESCE(finished_at, $4) ELSE finished_at END \
              WHERE id = $1 AND status = $2",
            run.as_uuid(),
            from.as_str(),
            to.as_str(),
            at,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }

        let exists = sqlx::query_scalar!("SELECT 1 FROM run WHERE id = $1", run.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_some();

        if exists {
            Ok(false)
        } else {
            Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            })
        }
    }

    /// The `run_step` twin of [`transition_run`](WriteStore::transition_run). Its terminal set is
    /// the narrower one of [`StepStatus::is_terminal`]: `failed` is **not** in it, because a
    /// failed step can still be promoted to a gate (§4.8).
    ///
    /// `awaiting_approval -> awaiting_approval` is the one self-move §4.3 sanctions, and it goes
    /// through this compare-and-set unremarkably: the `WHERE` matches the row it is already on.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }`; [`StoreError::Constraint`] for a pair
    /// outside [`StepStatus::can_move_to`], refused before the `UPDATE`.
    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        if let Err(refusal) = legal_move(from, to) {
            let exists =
                sqlx::query_scalar!("SELECT 1 FROM run_step WHERE id = $1", step.as_uuid())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(map_sqlx)?
                    .is_some();
            return refuse_illegal_move::<StepStatus, _>(exists, step, refusal);
        }

        let moved = sqlx::query!(
            "UPDATE run_step \
                SET status      = $3, \
                    started_at  = CASE WHEN $3 = 'running' \
                                       THEN COALESCE(started_at, $4) ELSE started_at END, \
                    finished_at = CASE WHEN $3 IN ('done','cancelled','superseded') \
                                       THEN COALESCE(finished_at, $4) ELSE finished_at END \
              WHERE id = $1 AND status = $2",
            step.as_uuid(),
            from.as_str(),
            to.as_str(),
            at,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }

        let exists = sqlx::query_scalar!("SELECT 1 FROM run_step WHERE id = $1", step.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_some();

        if exists {
            Ok(false)
        } else {
            Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })
        }
    }

    /// The settle columns of [`StepOutcome`], and **never** `status`: a step's status is moved by
    /// [`transition_step`](WriteStore::transition_step), by the gate or by the fan-out, and a
    /// process that merely finished has not decided which of those it was.
    ///
    /// `usage` and `trim_record` are the two columns this writer does not own. The usage summer
    /// ([`set_step_usage`](PgStore::set_step_usage)) and the prompt assembler
    /// ([`set_step_prompt`](PgStore::set_step_prompt)) write them earlier in the step's life, so a
    /// `None` here means "leave what they wrote" - `COALESCE($3, usage)`, the same idiom
    /// `set_step_usage` uses for its digest. Every other field overwrites, `None` included:
    /// `verify_outcome` of a re-run that did not verify is genuinely absent.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }` when the step does not exist and
    /// [`StoreError::Fenced`] when it does and its run does not carry `fence`'s lease, told apart
    /// by `step_exists`' follow-up read on a miss (`interrupt_step`'s shape). `set_step_usage`'s
    /// locking CTE (MOD-40 blueprint B2, MOD-77 plan D2): the step, then its run.
    async fn finish_step(
        &self,
        fence: StepFence,
        step: StepId,
        outcome: StepOutcome,
    ) -> Result<()> {
        let updated = sqlx::query!(
            "WITH locked AS ( \
                 SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
                  WHERE s.id = $1 \
                    AND r.lease_owner IS NOT DISTINCT FROM $8 \
                    FOR NO KEY UPDATE OF s FOR SHARE OF r) \
             UPDATE run_step \
                SET exit_code        = $2, \
                    usage            = COALESCE($3, usage), \
                    trim_record      = COALESCE($4, trim_record), \
                    verify_outcome   = $5, \
                    verify_exit_code = $6, \
                    finished_at      = $7 \
               FROM locked \
              WHERE run_step.id = locked.id",
            step.as_uuid(),
            outcome.exit_code,
            outcome.usage,
            outcome.trim_record,
            outcome.verify_outcome.map(VerifyOutcome::as_str),
            outcome.verify_exit_code,
            outcome.finished_at,
            fence.owner(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if updated == 1 {
            return Ok(());
        }
        Err(Box::pin(fenced_or_missing(&self.pool, step)).await)
    }

    /// Plan D89: one compare-and-set `UPDATE`, `running -> failed` with the note.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }` when there is no such step, told apart
    /// from "not running" by `step_exists`' follow-up read.
    async fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>) -> Result<bool> {
        let moved = sqlx::query!(
            "UPDATE run_step \
                SET status = 'failed', gate_note = $2, finished_at = COALESCE(finished_at, $3) \
              WHERE id = $1 AND status = 'running'",
            step.as_uuid(),
            note,
            at,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }
        let mut conn = self.pool.acquire().await.map_err(map_sqlx)?;
        step_exists(&mut conn, step).await?;
        Ok(false)
    }

    /// `R-ORCH-2`'s four answers, a compare-and-set on `awaiting_approval` rather than on the
    /// caller's idea of the status: the gate is answered from a pane that may have been open a
    /// while, and a step that moved on in the meantime must say `Ok(false)`, not overwrite.
    ///
    /// The status each answer settles the step at is decided in Rust, so §4.3's table and this
    /// mapping stay one sentence each: `Approved | Skipped -> done`, `Rejected -> failed`,
    /// `Retried -> superseded`. `gate_note` is assigned rather than `COALESCE`d - a second answer
    /// carrying no note clears the first one's - while `finished_at` is `COALESCE`d, because a step
    /// that already finished keeps the instant it finished at.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }` when no row has that id; the "not
    /// awaiting" answer is `Ok(false)`, which is why the miss needs the follow-up read.
    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let settled = match outcome {
            GateOutcome::Approved | GateOutcome::Skipped => StepStatus::Done,
            GateOutcome::Rejected => StepStatus::Failed,
            GateOutcome::Retried => StepStatus::Superseded,
        };

        let answered = sqlx::query!(
            "UPDATE run_step \
                SET status       = $2, \
                    gate_outcome = $3, \
                    gate_note    = $4, \
                    finished_at  = COALESCE(finished_at, $5) \
              WHERE id = $1 AND status = 'awaiting_approval'",
            step.as_uuid(),
            settled.as_str(),
            outcome.as_str(),
            note.as_deref(),
            at,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if answered == 1 {
            return Ok(true);
        }

        let exists = sqlx::query_scalar!("SELECT 1 FROM run_step WHERE id = $1", step.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_some();

        if exists {
            Ok(false)
        } else {
            Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })
        }
    }

    /// ANA-2 §4.5's bookkeeping, one transaction (plan D6): the whole slot is locked and validated
    /// before the first candidate is touched, so a refused selection writes nothing at all.
    ///
    /// The lock is taken over the **slot** in `id` order rather than over the winner first: two
    /// selections racing on one `(run, position, attempt)` with different winners would otherwise
    /// each hold the other's row and deadlock. A winner that is not in the slot at all is therefore
    /// looked up by a second, unlocked probe, which is the only way to tell plan D14's two answers
    /// apart - an id that names nothing is [`StoreError::NotFound`], an id that names a step of
    /// another position is [`StoreError::Constraint`].
    ///
    /// The losers' `CASE` is blueprint hazard H-13: `selected = false` is recorded for every
    /// candidate, but only `pending`, `awaiting_approval` and `done` move to `superseded` - a
    /// `failed` or `cancelled` loser keeps the status that says *why* it lost. The judge row
    /// (`fanout_index = -1`) gets the same treatment for the same reason: it is settled directly
    /// rather than through [`legal_move`], because §4.5 makes its `done` part of the winner's
    /// outcome and a judge still at `pending` - a human who answered the fan-out themselves - must
    /// not turn the selection into a refusal; but only `pending`, `running` and
    /// `awaiting_approval` are settled. §4.5's judge-failure path (`docs/ANA-2.md:858-862`) is why:
    /// a failed judge parks the run at `awaiting_approval` with the reason in `gate_note`, and
    /// *this method is the human's pick*, so settling it would be the `failed -> done` §4.3 rejects
    /// over the top of the reason that pick was made from. "Nothing is lost" is that sentence.
    ///
    /// The status set is in the `WHERE` rather than a `CASE` over both columns so an unsettleable
    /// judge is not written at all: `run_step` carries the `set_updated_at` trigger
    /// (`0001_init.sql:564-580`), a no-op `UPDATE` would still bump the column the §4.4 cache
    /// cursor rides on, and `MemStore`'s guard leaves the row whole.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }` for an unknown `winner`;
    /// [`StoreError::Constraint`] when `winner` is not a candidate of that `(run, position,
    /// attempt)` - the judge row included, `fanout_index >= 0` being what a candidate is - or is
    /// not yet `awaiting_approval | done`.
    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let slot = sqlx::query!(
            r#"
            SELECT id     AS "id: StepId",
                   status AS "status: StepStatus",
                   fanout_index
              FROM run_step
             WHERE run_id = $1 AND position = $2 AND attempt = $3
             ORDER BY id
               FOR UPDATE
            "#,
            run.as_uuid(),
            position,
            attempt,
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let Some(row) = slot.iter().find(|row| row.id == winner) else {
            let exists =
                sqlx::query_scalar!("SELECT 1 FROM run_step WHERE id = $1", winner.as_uuid())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(map_sqlx)?
                    .is_some();
            return if exists {
                Err(StoreError::Constraint(not_a_fanout_candidate(
                    winner, run, position, attempt,
                )))
            } else {
                Err(StoreError::NotFound {
                    entity: "run_step",
                    id: winner.to_string(),
                })
            };
        };
        if row.fanout_index < 0 {
            return Err(StoreError::Constraint(not_a_fanout_candidate(
                winner, run, position, attempt,
            )));
        }
        if !matches!(row.status, StepStatus::AwaitingApproval | StepStatus::Done) {
            return Err(StoreError::Constraint(winner_is_not_settled(
                winner, row.status,
            )));
        }

        sqlx::query!(
            "UPDATE run_step SET selected = true, status = 'done' WHERE id = $1",
            winner.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query!(
            "UPDATE run_step \
                SET selected = false, \
                    status   = CASE WHEN status IN ('pending','awaiting_approval','done') \
                                    THEN 'superseded' ELSE status END \
              WHERE run_id = $1 AND position = $2 AND attempt = $3 \
                AND fanout_index >= 0 AND id <> $4",
            run.as_uuid(),
            position,
            attempt,
            winner.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query!(
            "UPDATE run_step SET status = 'done', gate_note = $4 \
              WHERE run_id = $1 AND position = $2 AND attempt = $3 AND fanout_index < 0 \
                AND status IN ('pending','running','awaiting_approval')",
            run.as_uuid(),
            position,
            attempt,
            reason.as_deref(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)
    }

    /// §4.4's loop half: `pending | awaiting_approval | done -> superseded`, and nothing else.
    ///
    /// The refusal is the law's table and not a stale compare-and-set, so the follow-up read has
    /// to fetch the *status* rather than a bare `SELECT 1`: it is what
    /// [`illegal_move`] names in the sentence.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }`; [`StoreError::Constraint`] from any
    /// other status.
    async fn supersede_step(&self, step: StepId) -> Result<()> {
        let moved = sqlx::query!(
            "UPDATE run_step SET status = 'superseded' \
              WHERE id = $1 AND status IN ('pending','awaiting_approval','done')",
            step.as_uuid(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(());
        }

        let current = sqlx::query_scalar!(
            r#"SELECT status AS "status: StepStatus" FROM run_step WHERE id = $1"#,
            step.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        match current {
            Some(status) => Err(StoreError::Constraint(illegal_move(
                "run_step",
                status,
                StepStatus::Superseded,
            ))),
            None => Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            }),
        }
    }

    /// `run_step_tree` upserted on the table's own `(run_step_id, repo_id)` key (ANA-2 §4.6): a
    /// step re-preparing a tree it already has replaces the row rather than raising `23505`.
    ///
    /// The step is looked up **before** the batch is judged (plan D14: `NotFound` beats
    /// `Constraint`, the row first and legality second), which is the order
    /// [`MemStore`](htui_core::store::MemStore)'s `check_step_batch` uses: an unknown step carrying
    /// a stray row is `NotFound`, not the `Constraint` the row alone would earn.
    ///
    /// The batch is then checked whole before the first insert, [`WriteStore::append_events`]-style:
    /// a row naming another step is refused in Rust, because `run_step_tree.run_step_id` is bound
    /// from `step` and a stray row would otherwise be written *under the argument's key*, silently
    /// re-homing it. An unknown `repo_id` is the foreign key's `23503`, which [`map_sqlx`] turns
    /// into the same [`StoreError::Constraint`]; the transaction is what makes it write nothing.
    ///
    /// One statement per row rather than one `UNNEST`: a batch naming the same repo twice would be
    /// `ON CONFLICT DO UPDATE`'s "cannot affect row a second time", where
    /// [`MemStore`](htui_core::store::MemStore)'s map simply keeps the last. Batches are one row
    /// per repo in scope.
    ///
    /// An empty slice still runs the existence and fence check (`step_fence`) and writes nothing.
    /// The fence takes the step's update lock ([`StepLock::Update`], MOD-77 plan D1), because this
    /// transaction may go on to update `run_step.isolation_path`. It is taken before the run's
    /// share lock, `park_step`'s order.
    ///
    /// The transaction also writes `run_step.isolation_path` (ANA-2 `:903`, plan D33), because a
    /// step has many trees and one isolation path and this is the only call that sees both. The
    /// batch's primary repo wins, else its lowest `repo_id`; the `is_primary` flag is read inside
    /// the same transaction, so a repo promoted concurrently cannot change the answer halfway
    /// through. An empty batch names no tree and the `UPDATE` is skipped, which is what keeps
    /// `upsert_step_tree(step, &[])` a check rather than a write.
    ///
    /// `updated_at` is the `trg_run_step_updated_at` trigger's (`0001_init.sql:574-580`); no write
    /// path sets it by hand, because the §4.4 cache cursor rides on it.
    ///
    /// # Errors
    ///
    /// In this order: [`StoreError::NotFound`] `{ entity: "run_step" }`; [`StoreError::Fenced`]
    /// when the step's run has a `lease_owner` other than `fence`'s (MOD-41 plan D1);
    /// [`StoreError::Constraint`] when a row's `run_step_id` is not `step` or names an unknown
    /// repo.
    async fn upsert_step_tree(
        &self,
        fence: StepFence,
        step: StepId,
        trees: &[RunStepTree],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        step_fence(&mut tx, step, fence, StepLock::Update).await?;

        for row in trees {
            if row.run_step_id != step {
                return Err(StoreError::Constraint(row_names_another_step(
                    "run_step_tree",
                    row.run_step_id,
                    step,
                )));
            }
        }

        for row in trees {
            sqlx::query!(
                "INSERT INTO run_step_tree (run_step_id, repo_id, mode, path, base_ref, dirty) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (run_step_id, repo_id) DO UPDATE \
                    SET mode     = EXCLUDED.mode, \
                        path     = EXCLUDED.path, \
                        base_ref = EXCLUDED.base_ref, \
                        dirty    = EXCLUDED.dirty",
                step.as_uuid(),
                row.repo_id.as_uuid(),
                row.mode.as_str(),
                row.path,
                row.base_ref,
                row.dirty,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        // `run_step.isolation_path` (plan D33). The batch's primary repo wins, else its lowest
        // `repo_id`; both halves of that rule are one sort key, `(not primary, repo_id)`, so a
        // project carrying two primaries - which the schema permits - resolves the same way here
        // as it does on `MemStore` rather than to whichever row the caller listed first.
        if !trees.is_empty() {
            let repo_ids: Vec<Uuid> = trees.iter().map(|row| row.repo_id.as_uuid()).collect();
            let primaries = sqlx::query_scalar!(
                "SELECT id FROM repo WHERE id = ANY($1) AND is_primary",
                &repo_ids,
            )
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;

            if let Some(row) = trees
                .iter()
                .min_by_key(|row| (!primaries.contains(&row.repo_id.as_uuid()), row.repo_id))
            {
                sqlx::query!(
                    "UPDATE run_step SET isolation_path = $2 WHERE id = $1",
                    step.as_uuid(),
                    row.path,
                )
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            }
        }

        tx.commit().await.map_err(map_sqlx)
    }

    /// `R-ORCH-11`'s two hashes, on the same key and with the same refusals as
    /// [`upsert_step_tree`](WriteStore::upsert_step_tree) - see its doc for why the batch is
    /// checked whole and written row by row.
    ///
    /// `after_hash` is nullable and is assigned rather than `COALESCE`d: the git driver records the
    /// `before` hash when it takes the tree and the `after` hash when it commits, and a second call
    /// carrying `None` means the step produced no commit after all.
    ///
    /// The fence key-share-locks the step and then share-locks its run ([`StepLock::KeyShare`],
    /// MOD-77 plan D1, review L1): the lock the inserts' foreign keys take on the step anyway.
    ///
    /// # Errors
    ///
    /// In this order: [`StoreError::NotFound`] `{ entity: "run_step" }`; [`StoreError::Fenced`]
    /// when the step's run has a `lease_owner` other than `fence`'s (MOD-41 plan D1);
    /// [`StoreError::Constraint`] when a row's `run_step_id` is not `step` or names an unknown
    /// repo.
    async fn record_commits(
        &self,
        fence: StepFence,
        step: StepId,
        commits: &[RunStepCommit],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        step_fence(&mut tx, step, fence, StepLock::KeyShare).await?;

        for row in commits {
            if row.run_step_id != step {
                return Err(StoreError::Constraint(row_names_another_step(
                    "run_step_commit",
                    row.run_step_id,
                    step,
                )));
            }
        }

        for row in commits {
            sqlx::query!(
                "INSERT INTO run_step_commit (run_step_id, repo_id, before_hash, after_hash) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (run_step_id, repo_id) DO UPDATE \
                    SET before_hash = EXCLUDED.before_hash, \
                        after_hash  = EXCLUDED.after_hash",
                step.as_uuid(),
                row.repo_id.as_uuid(),
                row.before_hash,
                row.after_hash,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        tx.commit().await.map_err(map_sqlx)
    }

    /// One `command_run` row, every column the caller's (plan D31).
    ///
    /// A transaction for one insert, and not the bare `execute` a single statement would allow,
    /// because the step check is a second statement and the contract orders the two: an unknown
    /// step must be [`StoreError::NotFound`] and not the `23503` the insert alone would raise.
    /// [`step_exists`] is the same check the two batch upserts run, in the same place and for the
    /// same reason.
    ///
    /// No `RETURNING`: the row has no column the server fills in - `queued_at`'s `DEFAULT now()`
    /// is never reached, because the seam takes that instant from the caller like every other one
    /// (blueprint F-S) - so the stored row *is* the argument. Building it in Rust is both cheaper
    /// than a round trip and the only way to promise that `MemStore` answers identically.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }`; [`StoreError::Constraint`] for an
    /// unknown `box_id` (`23503`) and for an `id` the table already holds (`23505`).
    async fn record_command_run(&self, new: NewCommandRun) -> Result<CommandRun> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        step_exists(&mut tx, new.run_step_id).await?;

        sqlx::query!(
            "INSERT INTO command_run (id, run_step_id, box_id, class, command, cwd, status, \
             exit_code, output, queued_at, started_at, finished_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            new.id.as_uuid(),
            new.run_step_id.as_uuid(),
            new.box_id.as_uuid(),
            new.class,
            new.command,
            new.cwd,
            new.status.as_str(),
            new.exit_code,
            new.output,
            new.queued_at,
            new.started_at,
            new.finished_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(CommandRun::from(new))
    }

    /// A step's `command_run` rows in `(queued_at, id)` order; an unknown step reads empty.
    ///
    /// A `SELECT` in `pg::write` rather than beside its twin `step_trees` in `pg::read`, because
    /// the trait puts it on [`WriteStore`]: `command_run` is not a mirrored table, so there was
    /// nowhere else to carry a read the conformance suite - written against `WriteStore` alone -
    /// can reach.
    ///
    /// The `id` tiebreak is load-bearing. Two verify runs of one step can share a `queued_at`
    /// truncated to microseconds, and ids are UUIDv7, so ordering by `(queued_at, id)` breaks that
    /// tie by mint time rather than by whatever order the heap hands back.
    ///
    /// # Errors
    ///
    /// The backend's own failures only.
    async fn command_runs(&self, step: StepId) -> Result<Vec<CommandRun>> {
        sqlx::query_as!(
            CommandRun,
            r#"
            SELECT id          AS "id: CommandRunId",
                   run_step_id AS "run_step_id: StepId",
                   box_id      AS "box_id: BoxId",
                   class,
                   command,
                   cwd,
                   status      AS "status: CommandRunStatus",
                   exit_code,
                   output,
                   queued_at,
                   started_at,
                   finished_at
              FROM command_run WHERE run_step_id = $1 ORDER BY queued_at, id
            "#,
            step.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The document at `max(version) + 1` for its `(item, kind)`, allocated **under the item's row
    /// lock** (plan D6): two writers arriving together serialise on that lock, so the second one's
    /// `MAX(version)` already sees the first one's row and no version is ever handed out twice.
    ///
    /// The lock is on `item` rather than on `document`, because the row the second writer must wait
    /// for does not exist yet - there is nothing in `document` to lock. `SELECT ... FOR UPDATE` on
    /// the parent is the only row both writers are guaranteed to contend on.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "item" }`; [`StoreError::Constraint`] on a duplicate id
    /// or a `created_by` / `produced_by_step_id` that names no row (`23503`).
    async fn write_document(&self, new: NewDocument) -> Result<Document> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        sqlx::query_scalar!(
            "SELECT 1 FROM item WHERE id = $1 FOR UPDATE",
            new.item_id.as_uuid()
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: new.item_id.to_string(),
        })?;

        let written = insert_document(&mut tx, new).await?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(written)
    }

    /// §4.8's promotion, one transaction (plan D6): the step, its run and its item are lifted to
    /// `awaiting_approval` together, so no reader sees a gated step under a running item.
    ///
    /// The refusal is **this writer's own rule, not §4.3's table**: `running -> awaiting_approval`
    /// is a legal step move, but a step still running has produced nothing to promote. Only `failed`
    /// and `awaiting_approval` promote - the first is `R-ORCH-5`'s "take this one over by hand", the
    /// second is idempotence for a second click. [`step_is_not_promotable`] is the sentence, which
    /// is why [`legal_move`] is not called here.
    ///
    /// The run and the item are moved by compare-and-set rather than unconditionally: a run already
    /// at `awaiting_approval` (a second promotion in the same gate) matches nothing and is left
    /// alone, which is the same zero-rows-tolerated shape [`claim_run`](WriteStore::claim_run) uses
    /// for its item. The item's `closed_at` is cleared alongside, because
    /// [`transition`](WriteStore::transition) - the statement `MemStore` routes this through - does.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }`; [`StoreError::Constraint`] when the step
    /// is at any other status, or its run is terminal. Both are decided under the two row locks and
    /// before the first write.
    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let Some(row) = sqlx::query!(
            r#"
            SELECT s.status  AS "step_status: StepStatus",
                   r.id      AS "run_id: RunId",
                   r.status  AS "run_status: RunStatus",
                   r.item_id AS "item_id: ItemId"
              FROM run_step s JOIN run r ON r.id = s.run_id
             WHERE s.id = $1
               FOR UPDATE OF s, r
            "#,
            step.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        else {
            return Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            });
        };

        if !matches!(
            row.step_status,
            StepStatus::Failed | StepStatus::AwaitingApproval
        ) {
            return Err(StoreError::Constraint(step_is_not_promotable(
                step,
                row.step_status,
            )));
        }
        if row.run_status.is_terminal() {
            return Err(StoreError::Constraint(run_is_terminal(
                row.run_id,
                row.run_status,
            )));
        }

        sqlx::query!(
            "UPDATE run_step SET status = 'awaiting_approval', promoted_at = $2 WHERE id = $1",
            step.as_uuid(),
            at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query!(
            "UPDATE run SET status = 'awaiting_approval' WHERE id = $1 AND status = 'running'",
            row.run_id.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        if let Some(item) = row.item_id {
            sqlx::query!(
                "UPDATE item SET status = 'awaiting_approval', closed_at = NULL \
                  WHERE id = $1 AND status = 'in_progress'",
                item.as_uuid(),
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        tx.commit().await.map_err(map_sqlx)
    }

    /// MOD-37 M5: one `UPDATE`; `trg_run_step_updated_at` moves `updated_at`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] `{ entity: "run_step" }` when no row has that id.
    async fn record_opening(&self, step: StepId, opening: StepOpening) -> Result<()> {
        let written = sqlx::query!(
            "UPDATE run_step SET opening = $2 WHERE id = $1",
            step.as_uuid(),
            opening.as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        if written == 1 {
            Ok(())
        } else {
            Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })
        }
    }

    /// MOD-37 R-5: one fenced compare-and-set, `running -> done` with `gate_outcome = 'skipped'`.
    /// `finish_step`'s locking CTE (MOD-77 plan D2), with `status = 'running'` in the CTE, so a
    /// park committed first matches no row.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }`, then [`StoreError::Fenced`], told apart
    /// from "not running" by `fenced_miss` on a miss.
    async fn pass_step(
        &self,
        fence: StepFence,
        step: StepId,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let moved = sqlx::query!(
            "WITH locked AS ( \
                 SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
                  WHERE s.id = $1 AND s.status = 'running' \
                    AND r.lease_owner IS NOT DISTINCT FROM $4 \
                    FOR NO KEY UPDATE OF s FOR SHARE OF r) \
             UPDATE run_step \
                SET status = 'done', gate_outcome = 'skipped', \
                    gate_note = COALESCE($2, gate_note), \
                    finished_at = COALESCE(finished_at, $3) \
               FROM locked \
              WHERE run_step.id = locked.id AND run_step.status = 'running'",
            step.as_uuid(),
            note,
            at,
            fence.owner(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }
        Box::pin(fenced_miss(&self.pool, step, fence)).await
    }

    /// MOD-37 R-5: the gate's park, one transaction in `promote_step`'s shape. The step and its
    /// run are read under `FOR UPDATE`, and every refusal is decided before the first `UPDATE`,
    /// so a refused park rolls back having written nothing. The run and item statements are
    /// `promote_step`'s.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run_step" }`; [`StoreError::Fenced`].
    async fn park_step(&self, fence: StepFence, step: StepId) -> Result<ParkOutcome> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let Some(row) = sqlx::query!(
            r#"
            SELECT s.status      AS "step_status: StepStatus",
                   r.id          AS "run_id: RunId",
                   r.status      AS "run_status: RunStatus",
                   r.item_id     AS "item_id: ItemId",
                   r.lease_owner AS "lease_owner?"
              FROM run_step s JOIN run r ON r.id = s.run_id
             WHERE s.id = $1
               FOR UPDATE OF s, r
            "#,
            step.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        else {
            return Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            });
        };

        if row.lease_owner != fence.owner() {
            return Err(StoreError::Fenced { step });
        }
        if row.step_status != StepStatus::Running {
            return Ok(ParkOutcome::StepMoved);
        }
        if row.run_status != RunStatus::Running {
            return Ok(ParkOutcome::RunMoved);
        }

        sqlx::query!(
            "UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1",
            step.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query!(
            "UPDATE run SET status = 'awaiting_approval' WHERE id = $1 AND status = 'running'",
            row.run_id.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        if let Some(item) = row.item_id {
            sqlx::query!(
                "UPDATE item SET status = 'awaiting_approval', closed_at = NULL \
                  WHERE id = $1 AND status = 'in_progress'",
                item.as_uuid(),
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(ParkOutcome::Parked)
    }

    /// §4.3's failure row: `queued | running | awaiting_approval -> failed`, with the reason and
    /// the caller's instant.
    ///
    /// `finished_at` is `COALESCE`d rather than assigned, so a run that already stamped a finish
    /// (a `cancelled` racing a `failed` is not possible under the law, but a re-`finish` is) keeps
    /// the first instant. `failure` is assigned: the newest reason is the one the pane shows.
    ///
    /// The three statuses in the `WHERE` are exactly [`RunStatus::can_move_to`]'s `-> Failed`
    /// column, so the compare-and-set *is* the law, and like
    /// [`supersede_step`](WriteStore::supersede_step) the follow-up read fetches the **status**
    /// rather than a bare `SELECT 1`: it is what [`illegal_move`] names in the sentence.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run" }`; [`StoreError::Constraint`] when the run is
    /// already terminal.
    async fn fail_run(&self, run: RunId, failure: &str, at: DateTime<Utc>) -> Result<()> {
        let moved = sqlx::query!(
            "UPDATE run SET status = 'failed', failure = $2, \
                            finished_at = COALESCE(finished_at, $3) \
              WHERE id = $1 AND status IN ('queued','running','awaiting_approval')",
            run.as_uuid(),
            failure,
            at,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(());
        }

        let current = sqlx::query_scalar!(
            r#"SELECT status AS "status: RunStatus" FROM run WHERE id = $1"#,
            run.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        match current {
            Some(status) => Err(StoreError::Constraint(illegal_move(
                "run",
                status,
                RunStatus::Failed,
            ))),
            None => Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            }),
        }
    }

    /// Plan M2 D7: the run's terminal move and the item's mirror, one transaction.
    ///
    /// [`promote_step`](WriteStore::promote_step) is the template — the rows are taken
    /// `FOR UPDATE` and every refusal is decided under the locks and before the first `UPDATE` —
    /// with one addition that is the whole point of the writer. The item's row is locked
    /// **before** its sibling runs are counted, so two runs of one item finishing at the same
    /// instant serialise on it: without that lock each would see the other still live in its own
    /// snapshot, both would decline to move the item, and the item would be stranded
    /// `in_progress` with no live run left to move it.
    ///
    /// The lock order is `run` then `item`, which is `promote_step`'s, so the two cannot deadlock
    /// against each other.
    ///
    /// The item's target is derived in Rust rather than in SQL, from the same `match` `MemStore`
    /// runs, so ANA-2 §4.3's mapping is written once per rule and not once per backend; and
    /// `closed_at` is `to.is_terminal()` in SQL exactly as
    /// [`transition`](WriteStore::transition) writes it (`:603-611`). Zero rows affected by the
    /// final `UPDATE` is not an error: the item moved under us, which plan D17 says to leave alone.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "run" }`; [`StoreError::Constraint`] with
    /// [`finish_run_needs_a_terminal_status`] for a non-terminal `to`, with
    /// [`failure_disagrees_with_status`] when `failure` and `to` disagree, and with
    /// [`illegal_move`]'s sentence when the run is already terminal.
    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        finish_run_on(&mut tx, run, to, failure, at).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    /// `R-TUI-9`'s close-out: the summary, the commits and the item's move to `closed`, one
    /// transaction (plan D6), refused while any run of the item is still active.
    ///
    /// The item's row is taken `FOR UPDATE` first and held to the commit, which is what makes the
    /// three effects one act rather than three: it is the lock
    /// [`write_document`](WriteStore::write_document) needs to allocate the summary's version, and
    /// it is also what stops a [`claim_run`](WriteStore::claim_run) from admitting a run of this
    /// item between the live-run probe and the `UPDATE` - `claim_run` moves the item to
    /// `in_progress` and so contends on the same row.
    ///
    /// The live-run probe orders by `(queued_at, id)` rather than taking any row, so the run the
    /// sentence names is the same one [`MemStore`](htui_core::store::MemStore) names: with two
    /// live runs the backends would otherwise disagree about which is "in the way".
    ///
    /// Every commit row's step is probed by `step_exists`, because the foreign key would make an
    /// unknown step [`StoreError::Constraint`] where the contract says [`StoreError::NotFound`].
    /// An unknown **repo** is left to the key (`23503`), which [`map_sqlx`] turns into
    /// `Constraint`; it is raised at the insert rather than before it, and the transaction is what
    /// makes the difference unobservable - nothing has been committed when it fires.
    ///
    /// The insert is `insert_document` and not the trait method, which would open a transaction
    /// of its own; the closing `UPDATE` is [`transition`](WriteStore::transition)'s statement with
    /// `closed_at` fixed, since `closed` is terminal by construction here. Its `AND status = $2`
    /// cannot miss under the row lock, and is kept as the assertion that it cannot.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "item" }`, or `{ entity: "run_step" }` for a commit row
    /// naming a step that does not exist; [`StoreError::Constraint`] when a run of the item is
    /// `queued | running | awaiting_approval`, when `summary.kind` is not `summary`, when
    /// `summary.item_id` is not `item`, when `resolution` does not close the item's status under
    /// ANA-11 §4.2 ([`resolution_not_closable`]), when a commit row names an unknown repo, or when
    /// the summary duplicates a document id or names an unknown `created_by` /
    /// `produced_by_step_id`. No refusal writes anything.
    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let Some(status) = sqlx::query_scalar!(
            r#"SELECT status AS "status: Status" FROM item WHERE id = $1 FOR UPDATE"#,
            item.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        else {
            return Err(StoreError::NotFound {
                entity: "item",
                id: item.to_string(),
            });
        };

        if let Some(live) = sqlx::query!(
            r#"
            SELECT id     AS "id: RunId",
                   status AS "status: RunStatus"
              FROM run
             WHERE item_id = $1 AND status IN ('queued','running','awaiting_approval')
             ORDER BY queued_at, id
             LIMIT 1
            "#,
            item.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        {
            return Err(StoreError::Constraint(item_has_a_live_run(
                item,
                live.id,
                live.status,
            )));
        }

        if summary.kind != "summary" {
            return Err(StoreError::Constraint(close_out_needs_a_summary(
                &summary.kind,
            )));
        }
        if summary.item_id != item {
            return Err(StoreError::Constraint(summary_names_another_item(
                item,
                summary.item_id,
            )));
        }
        // ANA-11 §4.2's law, not §4.3's table: close-out is the only way into `closed` (MOD-38
        // PRD D1). `chk_item_resolution_iff_closed` would refuse a missing resolution; this refuses
        // a wrong one, before the first write.
        if !resolution.closes_from(status) {
            return Err(StoreError::Constraint(resolution_not_closable(
                item, status, resolution,
            )));
        }

        let mut probed: Vec<StepId> = Vec::new();
        for row in commits {
            if !probed.contains(&row.run_step_id) {
                step_exists(&mut tx, row.run_step_id).await?;
                probed.push(row.run_step_id);
            }
        }

        let written = insert_document(&mut tx, summary).await?;

        for row in commits {
            sqlx::query!(
                "INSERT INTO run_step_commit (run_step_id, repo_id, before_hash, after_hash) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (run_step_id, repo_id) DO UPDATE \
                    SET before_hash = EXCLUDED.before_hash, \
                        after_hash  = EXCLUDED.after_hash",
                row.run_step_id.as_uuid(),
                row.repo_id.as_uuid(),
                row.before_hash,
                row.after_hash,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        let closed = sqlx::query!(
            "UPDATE item SET status = 'closed', resolution = $3, closed_at = clock_timestamp() \
              WHERE id = $1 AND status = $2",
            item.as_uuid(),
            status.as_str(),
            resolution.as_str(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if closed != 1 {
            // Impossible for `create_run`'s reason: the item has been locked `FOR UPDATE` since
            // this transaction's first statement, so the status the close-out was decided against
            // is still the row's.
            return Err(StoreError::Backend(format!(
                "item `{item}` did not move `{status}` -> `closed` under the row lock this \
                 transaction holds: no rule of ANA-2 §4.3 can produce this"
            )));
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(written)
    }

    /// One `item_note`: ANA-2 invariant 7's refusal note, and the one thing a refusing gate always
    /// leaves behind.
    ///
    /// No guard of its own. Every refusal [`MemStore`](htui_core::store::MemStore) spells out is a
    /// key of the table - `item_note_pkey` on a repeated id, and the three foreign keys on
    /// `item_id`, `created_by` and `via_step_id` (the last added at `0001_init.sql:591`, which is
    /// what makes a dangling step a `23503` here rather than a silently stored orphan). [`map_sqlx`]
    /// turns all four into [`StoreError::Constraint`].
    ///
    /// `created_at` is the caller's (F-S), so the note a chat writes and the note the pane shows
    /// carry the instant the author wrote at rather than the instant the row landed.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] on an unknown item, author, box or step, or a duplicate id.
    async fn add_note(&self, note: NewNote) -> Result<Note> {
        sqlx::query_as!(
            Note,
            r#"
            INSERT INTO item_note (id, item_id, body, created_by, box_id, via_step_id, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id          AS "id: htui_core::model::NoteId",
                      item_id     AS "item_id: ItemId",
                      body,
                      created_by  AS "created_by: UserId",
                      box_id      AS "box_id: BoxId",
                      via_step_id AS "via_step_id: StepId",
                      created_at
            "#,
            note.id.as_uuid(),
            note.item_id.as_uuid(),
            note.body,
            note.created_by.as_uuid(),
            note.box_id.map(BoxId::as_uuid),
            note.via_step_id.map(StepId::as_uuid),
            note.created_at,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    // ---- ANA-11 §5.1 (MOD-38) ----

    /// The spec header's compare-and-set (plan D9), one statement per branch plus
    /// [`cas_miss`]'s follow-up read on a miss.
    ///
    /// `None` is an `INSERT ... ON CONFLICT DO NOTHING`, so a header that already exists is
    /// untouched and answered `Stale`; `Some(v)` is an `UPDATE ... WHERE version = v`, and a miss
    /// with no row at all is `NotFound`. `updated_at` is the column default on the insert and the
    /// trigger's on the update.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "requirement_spec" }` for `Some(_)` with no header;
    /// [`StoreError::Constraint`] for a project or owner that names no row (`23503`).
    async fn set_requirement_spec(
        &self,
        project: ProjectId,
        expected_version: Option<i32>,
        owner_id: UserId,
        preamble: String,
    ) -> Result<CasOutcome<RequirementSpec>> {
        let written = match expected_version {
            None => sqlx::query_as!(
                RequirementSpec,
                r#"
                INSERT INTO requirement_spec (project_id, owner_id, preamble)
                VALUES ($1, $2, $3)
                ON CONFLICT (project_id) DO NOTHING
                RETURNING project_id AS "project_id: ProjectId",
                          owner_id   AS "owner_id: UserId",
                          preamble,
                          version,
                          updated_at
                "#,
                project.as_uuid(),
                owner_id.as_uuid(),
                preamble,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
            Some(version) => sqlx::query_as!(
                RequirementSpec,
                r#"
                UPDATE requirement_spec
                   SET owner_id = $3, preamble = $4, version = version + 1
                 WHERE project_id = $1 AND version = $2
                RETURNING project_id AS "project_id: ProjectId",
                          owner_id   AS "owner_id: UserId",
                          preamble,
                          version,
                          updated_at
                "#,
                project.as_uuid(),
                version,
                owner_id.as_uuid(),
                preamble,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?,
        };

        match written {
            Some(row) => Ok(CasOutcome::Applied(row)),
            None => cas_miss(
                self.requirement_spec(project).await?,
                "requirement_spec",
                project,
            ),
        }
    }

    /// One `requirement_area` row. The code is checked here, before the insert, so the refusal
    /// is [`invalid_area_code`]'s sentence on both stores rather than the `CHECK`'s own text.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a code outside `^[A-Z][A-Z0-9]{1,15}$`, a code the project
    /// already has or a duplicate id (`23505`), or an unknown project (`23503`).
    async fn create_requirement_area(&self, new: NewRequirementArea) -> Result<RequirementArea> {
        if !RequirementArea::code_is_valid(&new.code) {
            return Err(StoreError::Constraint(invalid_area_code(&new.code)));
        }
        sqlx::query_as!(
            RequirementArea,
            r#"
            INSERT INTO requirement_area (id, project_id, code, title, description, position)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id         AS "id: RequirementAreaId",
                      project_id AS "project_id: ProjectId",
                      code,
                      title,
                      description,
                      position,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.project_id.as_uuid(),
            new.code,
            new.title,
            new.description,
            new.position,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// Mints a requirement: counter upsert, row insert and revision 1 in one statement, the
    /// shape of [`mint_item`](WriteStore::mint_item) (plan D8).
    ///
    /// `a` is the area row, and every later CTE selects from it, so an unknown area makes the
    /// whole statement insert nothing and return zero rows - which is exactly the `NotFound`.
    /// A refusal inside the statement (`23503` on `created_by` / `box_id`, `23505` on the id)
    /// rolls the counter upsert back with it, so no number is burned. The project, the code and
    /// the number all come from the area and its counter, never from the caller.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "requirement_area" }` for an unknown area;
    /// [`StoreError::Constraint`] for a duplicate id or a `created_by` / `box_id` that names no
    /// row.
    async fn mint_requirement(
        &self,
        area: RequirementAreaId,
        new: NewRequirement,
    ) -> Result<Requirement> {
        let minted = sqlx::query_as!(
            Requirement,
            r#"
            WITH a AS (
                SELECT id, project_id, code FROM requirement_area WHERE id = $1
            ), c AS (
                INSERT INTO requirement_key_counter (area_id, last_value)
                SELECT id, 1 FROM a
                ON CONFLICT (area_id)
                DO UPDATE SET last_value = requirement_key_counter.last_value + 1
                RETURNING last_value
            ), r AS (
                INSERT INTO requirement (id, project_id, area_id, area_code, number, body,
                                         rationale, priority, created_by)
                SELECT $2, a.project_id, a.id, a.code, c.last_value, $3, $4, $5, $6 FROM a, c
                RETURNING id, project_id, area_id, area_code, number, key, body, rationale,
                          priority, state, version, created_by, created_at, updated_at
            ), v AS (
                INSERT INTO requirement_revision (requirement_id, version, body, rationale,
                                                  priority, state, author_id, box_id, reason)
                SELECT id, version, body, rationale, priority, state, $6, $7, 'created' FROM r
                RETURNING requirement_id
            )
            SELECT r.id         AS "id!: RequirementId",
                   r.project_id AS "project_id!: ProjectId",
                   r.area_id    AS "area_id!: RequirementAreaId",
                   r.area_code  AS "area_code!",
                   r.number     AS "number!",
                   r.key        AS "key!",
                   r.body       AS "body!",
                   r.rationale  AS "rationale!",
                   r.priority   AS "priority!: Priority",
                   r.state      AS "state!: RequirementState",
                   r.version    AS "version!",
                   r.created_by AS "created_by!: UserId",
                   r.created_at AS "created_at!",
                   r.updated_at AS "updated_at!"
              FROM r, v
            "#,
            area.as_uuid(),
            new.id.as_uuid(),
            new.body,
            new.rationale,
            new.priority.as_str(),
            new.created_by.as_uuid(),
            new.box_id.map(BoxId::as_uuid),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        minted.ok_or_else(|| StoreError::NotFound {
            entity: "requirement_area",
            id: area.to_string(),
        })
    }

    /// [`revise_requirement`] with the patch's columns and reason, and an `amends` citation.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "requirement" }`; [`StoreError::Constraint`] for a
    /// withdrawn requirement ([`requirement_withdrawn`]) or an `amended_by` / author / box that
    /// names no row.
    async fn amend_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        patch: RequirementPatch,
        amended_by: ItemId,
    ) -> Result<RequirementUpdate> {
        revise_requirement(
            &self.pool,
            id,
            expected_version,
            RequirementEdit {
                body: patch.body,
                rationale: patch.rationale,
                priority: patch.priority,
                state: None,
                author_id: patch.author_id,
                box_id: patch.box_id,
                reason: patch.reason,
                decided_by: amended_by,
                kind: CitationKind::Amends,
            },
        )
        .await
    }

    /// [`revise_requirement`] to `state = withdrawn`, reason `withdrawn`, and a `withdraws`
    /// citation.
    ///
    /// # Errors
    ///
    /// As [`amend_requirement`](WriteStore::amend_requirement), an already-withdrawn requirement
    /// included.
    async fn withdraw_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        withdrawn_by: ItemId,
        author_id: UserId,
        box_id: Option<BoxId>,
    ) -> Result<RequirementUpdate> {
        revise_requirement(
            &self.pool,
            id,
            expected_version,
            RequirementEdit {
                body: None,
                rationale: None,
                priority: None,
                state: Some(RequirementState::Withdrawn),
                author_id,
                box_id,
                reason: "withdrawn".to_owned(),
                decided_by: withdrawn_by,
                kind: CitationKind::Withdraws,
            },
        )
        .await
    }

    /// Upserts a live citation at the requirement's current version (plan D10), one transaction.
    ///
    /// The requirement is read `FOR SHARE`, which a concurrent amend or withdraw's `FOR UPDATE`
    /// waits on: the state the withdrawn rule is decided against and the version the row is
    /// stamped with are the ones the citation commits beside. The upsert revives a tombstone and
    /// overwrites `proposed_by_step_id`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "item" }` then `{ entity: "requirement" }`;
    /// [`StoreError::Constraint`] for an `addresses` / `reserves` citation of a withdrawn
    /// requirement ([`withdrawn_requirement_cited`]) or a step that names no row (`23503`).
    async fn cite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        proposed_by: Option<StepId>,
    ) -> Result<ItemRequirement> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        sqlx::query_scalar!("SELECT 1 FROM item WHERE id = $1", item.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or_else(|| StoreError::NotFound {
                entity: "item",
                id: item.to_string(),
            })?;
        let Some(cited) = sqlx::query!(
            r#"
            SELECT key     AS "key!",
                   state   AS "state: RequirementState",
                   version
              FROM requirement WHERE id = $1 FOR SHARE
            "#,
            requirement.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        else {
            return Err(StoreError::NotFound {
                entity: "requirement",
                id: requirement.to_string(),
            });
        };
        if cited.state == RequirementState::Withdrawn
            && matches!(kind, CitationKind::Addresses | CitationKind::Reserves)
        {
            return Err(StoreError::Constraint(withdrawn_requirement_cited(
                &cited.key, kind,
            )));
        }

        let row = sqlx::query_as!(
            ItemRequirement,
            r#"
            INSERT INTO item_requirement (item_id, requirement_id, kind, requirement_version,
                                          proposed_by_step_id)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (item_id, requirement_id, kind) DO UPDATE
               SET requirement_version = EXCLUDED.requirement_version,
                   proposed_by_step_id = EXCLUDED.proposed_by_step_id,
                   deleted_at          = NULL
            RETURNING item_id             AS "item_id: ItemId",
                      requirement_id      AS "requirement_id: RequirementId",
                      kind                AS "kind: CitationKind",
                      requirement_version,
                      proposed_by_step_id AS "proposed_by_step_id: StepId",
                      created_at,
                      updated_at,
                      deleted_at
            "#,
            item.as_uuid(),
            requirement.as_uuid(),
            kind.as_str(),
            cited.version,
            proposed_by.map(StepId::as_uuid),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(row)
    }

    /// Tombstones a live citation: `deleted_at` is set, the row stays (plan D10).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "item_requirement", id: citation_key(..) }` when no
    /// live row matches, a tombstone included.
    async fn uncite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<()> {
        let tombstoned = sqlx::query!(
            "UPDATE item_requirement SET deleted_at = clock_timestamp() \
              WHERE item_id = $1 AND requirement_id = $2 AND kind = $3 AND deleted_at IS NULL",
            item.as_uuid(),
            requirement.as_uuid(),
            kind.as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if tombstoned == 0 {
            return Err(StoreError::NotFound {
                entity: "item_requirement",
                id: citation_key(item, requirement, kind),
            });
        }
        Ok(())
    }

    /// Re-stamps a live citation at the requirement's current version, which clears `suspect`
    /// (plan D11), one transaction.
    ///
    /// The requirement is read `FOR SHARE`, as in [`cite`](WriteStore::cite), so the withdrawn
    /// rule is decided against the state the re-stamp commits beside.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "item_requirement", id: citation_key(..) }` when no
    /// live row matches, a tombstone included; then [`StoreError::Constraint`] for an
    /// `addresses` / `reserves` citation of a withdrawn requirement
    /// ([`withdrawn_requirement_cited`]), which `cite` would not stamp either (plan D10).
    async fn reconfirm(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<ItemRequirement> {
        let not_found = || StoreError::NotFound {
            entity: "item_requirement",
            id: citation_key(item, requirement, kind),
        };
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let cited = sqlx::query!(
            r#"
            SELECT r.key   AS "key!",
                   r.state AS "state: RequirementState"
              FROM item_requirement ir
              JOIN requirement r ON r.id = ir.requirement_id
             WHERE ir.item_id = $1 AND ir.requirement_id = $2 AND ir.kind = $3
               AND ir.deleted_at IS NULL
               FOR SHARE OF r
            "#,
            item.as_uuid(),
            requirement.as_uuid(),
            kind.as_str(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(not_found)?;
        if cited.state == RequirementState::Withdrawn
            && matches!(kind, CitationKind::Addresses | CitationKind::Reserves)
        {
            return Err(StoreError::Constraint(withdrawn_requirement_cited(
                &cited.key, kind,
            )));
        }

        let row = sqlx::query_as!(
            ItemRequirement,
            r#"
            UPDATE item_requirement ir
               SET requirement_version = r.version
              FROM requirement r
             WHERE r.id = ir.requirement_id
               AND ir.item_id = $1 AND ir.requirement_id = $2 AND ir.kind = $3
               AND ir.deleted_at IS NULL
            RETURNING ir.item_id             AS "item_id!: ItemId",
                      ir.requirement_id      AS "requirement_id!: RequirementId",
                      ir.kind                AS "kind!: CitationKind",
                      ir.requirement_version AS "requirement_version!",
                      ir.proposed_by_step_id AS "proposed_by_step_id?: StepId",
                      ir.created_at          AS "created_at!",
                      ir.updated_at          AS "updated_at!",
                      ir.deleted_at          AS "deleted_at?"
            "#,
            item.as_uuid(),
            requirement.as_uuid(),
            kind.as_str(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(not_found)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(row)
    }

    // -- MOD-42: the permission and control relay (plan D1-D5, D12-D14); the bodies are
    // `pg/relay.rs`'s (blueprint §2.10).

    async fn open_permission(&self, open: OpenPermission) -> Result<PermissionId> {
        super::relay::open_permission(self, open).await
    }

    async fn permission(&self, id: PermissionId) -> Result<Option<StepPermission>> {
        super::relay::permission(self, id).await
    }

    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> Result<Option<PermissionChoice>> {
        super::relay::apply_permission(self, id, owner).await
    }

    async fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> Result<u64> {
        super::relay::settle_permissions(self, session, to).await
    }

    async fn request_cancel(
        &self,
        run: RunId,
        user: UserId,
        box_id: BoxId,
    ) -> Result<CancelRequest> {
        super::relay::request_cancel(self, run, user, box_id).await
    }

    async fn pending_commands(&self, owner: Uuid, box_id: BoxId) -> Result<Vec<RunCommand>> {
        super::relay::pending_commands(self, owner, box_id).await
    }

    async fn resolve_command(
        &self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
    ) -> Result<bool> {
        super::relay::resolve_command(self, id, to, resolution).await
    }

    async fn relay_view(&self, item: ItemId) -> Result<RelayView> {
        super::relay::relay_view(self, item).await
    }

    async fn answer_permission(
        &self,
        id: PermissionId,
        option_id: &str,
        user: UserId,
        box_id: BoxId,
    ) -> Result<AnswerOutcome> {
        super::relay::answer_permission(self, id, option_id, user, box_id).await
    }

    // ---- MOD-70 (plan D1-D5, D9): follow-ups, in `relay.rs` ----

    async fn request_follow_up(&self, new: NewFollowUp) -> Result<FollowUpRequest> {
        super::relay::request_follow_up(self, new).await
    }

    async fn open_follow_ups(
        &self,
        run: RunId,
        step: StepId,
        session: RelaySessionId,
        owner: Uuid,
    ) -> Result<bool> {
        super::relay::open_follow_ups(self, run, step, session, owner).await
    }

    async fn next_follow_up(
        &self,
        step: StepId,
        session: RelaySessionId,
    ) -> Result<Option<QueuedFollowUp>> {
        super::relay::next_follow_up(self, step, session).await
    }

    async fn settle_follow_up(
        &self,
        id: RunCommandId,
        owner: Uuid,
        to: FollowUpSettle,
    ) -> Result<SettleOutcome> {
        super::relay::settle_follow_up(self, id, owner, to).await
    }

    async fn close_follow_ups(
        &self,
        step: StepId,
        session: RelaySessionId,
        reason: &str,
    ) -> Result<u64> {
        super::relay::close_follow_ups(self, step, session, reason).await
    }

    async fn close_dropped_follow_ups(&self, run: RunId, owner: Uuid, reason: &str) -> Result<u64> {
        super::relay::close_dropped_follow_ups(self, run, owner, reason).await
    }

    // ---- MOD-11 (D13) ----

    /// D13: [`write_document`](WriteStore::write_document) for the step's own item, one
    /// transaction: [`step_scope`] (the step and its run, `FOR SHARE OF s, r`), the own-item
    /// check, then the item `FOR UPDATE` and [`insert_document`] — `park_step`'s step → run → item
    /// order, so the two never deadlock
    /// (`pg_criteria.rs::a_step_document_racing_a_park_never_deadlocks`).
    ///
    /// # Errors
    ///
    /// As [`WriteStore::write_step_document`].
    async fn write_step_document(&self, fence: StepFence, new: NewDocument) -> Result<Document> {
        let Some(step) = new.produced_by_step_id else {
            return Err(StoreError::Constraint(document_needs_a_step()));
        };
        if let Some(refusal) = step_document_refusal(&new) {
            return Err(StoreError::Constraint(refusal));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let scope = step_scope(&mut tx, step, fence).await?;
        if scope.item != Some(new.item_id) {
            return Err(StoreError::Constraint(step_writes_own_item(
                step,
                new.item_id,
            )));
        }

        sqlx::query_scalar!(
            "SELECT 1 FROM item WHERE id = $1 FOR UPDATE",
            new.item_id.as_uuid()
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: new.item_id.to_string(),
        })?;

        let written = insert_document(&mut tx, new).await?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(written)
    }

    /// D13: [`add_note`](WriteStore::add_note)'s insert after [`step_scope`] and the own-item
    /// check, one transaction. No explicit item lock: the foreign key's `FOR KEY SHARE` on `item`
    /// comes after the run, which keeps the order.
    ///
    /// # Errors
    ///
    /// As [`WriteStore::add_step_note`].
    async fn add_step_note(&self, fence: StepFence, note: NewNote) -> Result<Note> {
        let Some(step) = note.via_step_id else {
            return Err(StoreError::Constraint(note_needs_a_step()));
        };
        if let Some(refusal) = step_note_refusal(&note) {
            return Err(StoreError::Constraint(refusal));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let scope = step_scope(&mut tx, step, fence).await?;
        if scope.item != Some(note.item_id) {
            return Err(StoreError::Constraint(step_writes_own_item(
                step,
                note.item_id,
            )));
        }

        let row = sqlx::query_as!(
            Note,
            r#"
            INSERT INTO item_note (id, item_id, body, created_by, box_id, via_step_id, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id          AS "id: htui_core::model::NoteId",
                      item_id     AS "item_id: ItemId",
                      body,
                      created_by  AS "created_by: UserId",
                      box_id      AS "box_id: BoxId",
                      via_step_id AS "via_step_id: StepId",
                      created_at
            "#,
            note.id.as_uuid(),
            note.item_id.as_uuid(),
            note.body,
            note.created_by.as_uuid(),
            note.box_id.map(BoxId::as_uuid),
            note.via_step_id.map(StepId::as_uuid),
            note.created_at,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(row)
    }

    /// D13, B-6: the upsert, one transaction after [`step_scope`], the own-item check and `to`'s
    /// project. A live row keeps its proposer; a tombstone revives with the step as proposer.
    /// `updated_at` is the `BEFORE UPDATE` trigger's.
    ///
    /// # Errors
    ///
    /// As [`WriteStore::propose_link`].
    async fn propose_link(&self, fence: StepFence, link: ProposeLink) -> Result<ItemLink> {
        if link.from == link.to {
            return Err(StoreError::Constraint(self_link(link.from)));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let scope = step_scope(&mut tx, link.step, fence).await?;
        if scope.item != Some(link.from) {
            return Err(StoreError::Constraint(step_writes_own_item(
                link.step, link.from,
            )));
        }
        let project = sqlx::query_scalar!(
            r#"SELECT project_id AS "project_id: ProjectId" FROM item WHERE id = $1"#,
            link.to.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: link.to.to_string(),
        })?;
        if project != scope.project {
            return Err(StoreError::Constraint(link_outside_project(link.to)));
        }

        let row = sqlx::query_as!(
            ItemLink,
            r#"
            INSERT INTO item_link (from_item_id, to_item_id, kind, proposed_by_step_id)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (from_item_id, to_item_id, kind) DO UPDATE
               SET proposed_by_step_id = CASE WHEN item_link.deleted_at IS NULL
                                              THEN item_link.proposed_by_step_id
                                              ELSE EXCLUDED.proposed_by_step_id END,
                   deleted_at          = NULL
            RETURNING from_item_id        AS "from_item_id: ItemId",
                      to_item_id          AS "to_item_id: ItemId",
                      kind                AS "kind: LinkKind",
                      proposed_by_step_id AS "proposed_by_step_id: StepId",
                      created_at,
                      updated_at,
                      deleted_at
            "#,
            link.from.as_uuid(),
            link.to.as_uuid(),
            link.kind.as_str(),
            link.step.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(row)
    }

    /// D13, B-5: tombstones the live link when a step of `link.step`'s run proposed it, one
    /// transaction after [`step_scope`] and the own-item check; a miss is told apart by one
    /// re-read of the live row.
    ///
    /// # Errors
    ///
    /// As [`WriteStore::withdraw_link`].
    async fn withdraw_link(&self, fence: StepFence, link: WithdrawLink) -> Result<ItemLink> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let scope = step_scope(&mut tx, link.step, fence).await?;
        if scope.item != Some(link.from) {
            return Err(StoreError::Constraint(step_writes_own_item(
                link.step, link.from,
            )));
        }

        let row = sqlx::query_as!(
            ItemLink,
            r#"
            UPDATE item_link SET deleted_at = clock_timestamp()
             WHERE from_item_id = $1 AND to_item_id = $2 AND kind = $3 AND deleted_at IS NULL
               AND proposed_by_step_id IN (SELECT id FROM run_step WHERE run_id = $4)
            RETURNING from_item_id        AS "from_item_id: ItemId",
                      to_item_id          AS "to_item_id: ItemId",
                      kind                AS "kind: LinkKind",
                      proposed_by_step_id AS "proposed_by_step_id: StepId",
                      created_at,
                      updated_at,
                      deleted_at
            "#,
            link.from.as_uuid(),
            link.to.as_uuid(),
            link.kind.as_str(),
            scope.run.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let Some(row) = row else {
            let live = sqlx::query_scalar!(
                "SELECT 1 FROM item_link \
                  WHERE from_item_id = $1 AND to_item_id = $2 AND kind = $3 AND deleted_at IS NULL",
                link.from.as_uuid(),
                link.to.as_uuid(),
                link.kind.as_str(),
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            let key = link_key(link.from, link.to, link.kind);
            return Err(if live.is_some() {
                StoreError::Constraint(link_not_proposed_by_run(&key))
            } else {
                StoreError::NotFound {
                    entity: "item_link",
                    id: key,
                }
            });
        };

        tx.commit().await.map_err(map_sqlx)?;
        Ok(row)
    }

    /// B-4: the item of `project` keyed `key`, on the pool. A key holding a NUL is `None` before
    /// the query, which Postgres would fail on the parameter (`22021`).
    ///
    /// # Errors
    ///
    /// The backend's own failures only.
    async fn item_by_key(&self, project: ProjectId, key: &str) -> Result<Option<ItemId>> {
        if key.contains('\0') {
            return Ok(None);
        }
        sqlx::query_scalar!(
            r#"SELECT id AS "id: ItemId" FROM item WHERE project_id = $1 AND key = $2"#,
            project.as_uuid(),
            key,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// MOD-78 D1: one plain read on the pool. There is no `FOR SHARE`: the answer is a snapshot
    /// either way, and a lock would queue behind `claim_run`'s and the fenced writes' row locks.
    /// The predicate is the fenced writes' `IS NOT DISTINCT FROM`.
    ///
    /// # Errors
    ///
    /// `NotFound { entity: "run" }` when no row matches; the backend's own failures.
    async fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool> {
        sqlx::query_scalar!(
            r#"SELECT lease_owner IS NOT DISTINCT FROM $2 AS "holds!" FROM run WHERE id = $1"#,
            run.as_uuid(),
            fence.owner(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "run",
            id: run.to_string(),
        })
    }

    /// D14, B-16: the shape refusal, then [`WriteStore::record_command_run`]'s check and insert
    /// with the heartbeat stamped `clock_timestamp()` in the same statement (R-3: a waiter that
    /// dies before its first claim is reaped like one that dies later). Like
    /// `record_command_run`, it takes no lock on `run` or `run_step` beyond the foreign key's
    /// `FOR KEY SHARE`, so no `park_step` cycle (T1's 40P01).
    ///
    /// # Errors
    ///
    /// `Constraint(command_not_queued())`, then `record_command_run`'s.
    async fn enqueue_command(&self, new: NewCommandRun) -> Result<CommandRun> {
        if new.status != CommandRunStatus::Queued
            || new.started_at.is_some()
            || new.finished_at.is_some()
            || new.exit_code.is_some()
            || new.output.is_some()
        {
            return Err(StoreError::Constraint(command_not_queued()));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        step_exists(&mut tx, new.run_step_id).await?;
        sqlx::query!(
            "INSERT INTO command_run (id, run_step_id, box_id, class, command, cwd, status, \
             queued_at, heartbeat_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, clock_timestamp())",
            new.id.as_uuid(),
            new.run_step_id.as_uuid(),
            new.box_id.as_uuid(),
            new.class,
            new.command,
            new.cwd,
            new.status.as_str(),
            new.queued_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(CommandRun::from(new))
    }

    /// D14 (blueprint §2.4): one transaction. The row `FOR UPDATE` decides `NotFound` and the
    /// not-`queued` refusal, and is beaten (R-3: its waiter is alive); then
    /// `pg_advisory_xact_lock` on the `(box, class)` pair, so two claimants of one pair count and
    /// admit one at a time across processes; then the pair's stale `running` rows are failed
    /// (OQ-3) and its stale `queued` rows cancelled (R-3), and the row is admitted when fewer
    /// than `limit` run and it is the oldest `queued`. A claim locks no `run` or `item` row; two
    /// claimants of different rows of one pair each hold only their own row when they meet on
    /// the advisory lock, so there is no cycle — and the `queued` reap skips a row another
    /// claimant holds (`SKIP LOCKED`: that waiter is alive), so it never waits on one. The reap
    /// commits whether or not the row is admitted.
    ///
    /// # Errors
    ///
    /// `NotFound { entity: "command_run" }`; `Constraint(command_not_claimable(status))`.
    async fn claim_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
        limit: u32,
    ) -> Result<Option<CommandRun>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query!(
            r#"SELECT box_id AS "box_id: BoxId", class, status AS "status: CommandRunStatus"
                 FROM command_run WHERE id = $1 FOR UPDATE"#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "command_run",
            id: id.to_string(),
        })?;
        if row.status != CommandRunStatus::Queued {
            return Err(StoreError::Constraint(command_not_claimable(row.status)));
        }
        // R-3: asking is the queued row's beat.
        sqlx::query!(
            "UPDATE command_run SET heartbeat_at = clock_timestamp() WHERE id = $1",
            id.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query!(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            format!("{}/{}", row.box_id, row.class),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let stale_after = COMMAND_STALE_AFTER.num_microseconds().unwrap_or(i64::MAX);
        sqlx::query!(
            "UPDATE command_run \
                SET status = 'failed', finished_at = clock_timestamp(), \
                    output = COALESCE(output || E'\\n', '') || $3 \
              WHERE box_id = $1 AND class = $2 AND status = 'running' \
                AND COALESCE(heartbeat_at, started_at, queued_at) \
                    < clock_timestamp() - $4::bigint * interval '1 microsecond'",
            row.box_id.as_uuid(),
            row.class,
            reaped_note(),
            stale_after,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        // R-3: the pair's `queued` rows whose waiter stopped asking. A `NULL` heartbeat (a row
        // `record_command_run` wrote `queued`) is never stale.
        sqlx::query!(
            "UPDATE command_run \
                SET status = 'cancelled', finished_at = clock_timestamp(), \
                    output = COALESCE(output || E'\\n', '') || $3 \
              WHERE id IN (SELECT id FROM command_run \
                            WHERE box_id = $1 AND class = $2 AND status = 'queued' \
                              AND heartbeat_at \
                                  < clock_timestamp() - $4::bigint * interval '1 microsecond' \
                              FOR UPDATE SKIP LOCKED)",
            row.box_id.as_uuid(),
            row.class,
            reaped_note(),
            stale_after,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let running = sqlx::query_scalar!(
            r#"SELECT count(*) AS "running!" FROM command_run
                WHERE box_id = $1 AND class = $2 AND status = 'running'"#,
            row.box_id.as_uuid(),
            row.class,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let oldest = sqlx::query_scalar!(
            r#"SELECT id AS "id: CommandRunId" FROM command_run
                WHERE box_id = $1 AND class = $2 AND status = 'queued'
                ORDER BY queued_at, id LIMIT 1"#,
            row.box_id.as_uuid(),
            row.class,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if running >= i64::from(limit.max(1)) || oldest != Some(id) {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(None);
        }

        let admitted = sqlx::query_as!(
            CommandRun,
            r#"
            UPDATE command_run
               SET status = 'running', claimed_by = $2, started_at = clock_timestamp(),
                   heartbeat_at = clock_timestamp()
             WHERE id = $1
            RETURNING id          AS "id: CommandRunId",
                      run_step_id AS "run_step_id: StepId",
                      box_id      AS "box_id: BoxId",
                      class,
                      command,
                      cwd,
                      status      AS "status: CommandRunStatus",
                      exit_code,
                      output,
                      queued_at,
                      started_at,
                      finished_at
            "#,
            id.as_uuid(),
            claimant,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(Some(admitted))
    }

    /// D14: the claimant's beat, one statement on the pool.
    ///
    /// # Errors
    ///
    /// The backend's own failures only.
    async fn beat_command(&self, id: CommandRunId, claimant: Uuid) -> Result<bool> {
        let done = sqlx::query!(
            "UPDATE command_run SET heartbeat_at = clock_timestamp() \
              WHERE id = $1 AND claimed_by = $2 AND status = 'running'",
            id.as_uuid(),
            claimant,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(done.rows_affected() == 1)
    }

    /// D14: the claimant's end of a row, one statement on the pool; the status is refused first.
    ///
    /// # Errors
    ///
    /// `Constraint(command_finish_status(status))` for `queued | running`.
    async fn finish_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
        status: CommandRunStatus,
        exit_code: Option<i32>,
        output: Option<String>,
    ) -> Result<bool> {
        if matches!(status, CommandRunStatus::Queued | CommandRunStatus::Running) {
            return Err(StoreError::Constraint(command_finish_status(status)));
        }
        let done = sqlx::query!(
            "UPDATE command_run \
                SET status = $3, exit_code = $4, output = $5, finished_at = clock_timestamp() \
              WHERE id = $1 AND claimed_by = $2 AND status = 'running'",
            id.as_uuid(),
            claimant,
            status.as_str(),
            exit_code,
            output,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(done.rows_affected() == 1)
    }

    /// D14: `queued | running → cancelled`; zero rows re-read the id to tell `NotFound` from a
    /// terminal row.
    ///
    /// # Errors
    ///
    /// `NotFound { entity: "command_run" }`.
    async fn cancel_command(&self, id: CommandRunId) -> Result<bool> {
        let done = sqlx::query!(
            "UPDATE command_run SET status = 'cancelled', finished_at = clock_timestamp() \
              WHERE id = $1 AND status IN ('queued', 'running')",
            id.as_uuid(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if done.rows_affected() == 1 {
            return Ok(true);
        }
        sqlx::query_scalar!("SELECT 1 FROM command_run WHERE id = $1", id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or_else(|| StoreError::NotFound {
                entity: "command_run",
                id: id.to_string(),
            })?;
        Ok(false)
    }

    async fn open_permissions(&self, scope: &Scope) -> Result<Vec<WaitingPermission>> {
        super::relay::open_permissions(self, scope).await
    }
}

/// The thirty-five rows a project is born with (MOD-15 D9/D10), on the caller's transaction.
///
/// Graphs first, then each graph's phases, then the kind (`item_kind.default_graph_id` is
/// `NOT NULL REFERENCES step_graph`, `0001_init.sql:288`), then the templates, which reference
/// only the project and its creator. Every non-timestamp column is bound — `gate_hard` and
/// `input_kinds` included, where they happen to equal the column default — so [`seed::KINDS`] is
/// the whole truth and a later migration's default cannot re-seed by omission.
///
/// The timestamp columns are the database's: `created_at` and `updated_at` on `step_graph` and
/// `prompt_template`, `updated_at` alone on `step_graph_phase` and `item_kind`, all `DEFAULT now()`
/// (`0001_init.sql:214-295`); the `now` the row constructors take is discarded here. No
/// `RETURNING`: nothing reads a seed row before the commit.
///
/// None of [`create_step_graph`](WriteStore::create_step_graph),
/// [`create_phase`](WriteStore::create_phase) or
/// [`create_item_kind`](WriteStore::create_item_kind) is reused: each runs on the pool rather than
/// on this transaction, so `create_item_kind`'s `EXISTS` guard would read through the pool, where
/// the graph inserted two statements ago is still invisible.
async fn seed_project(
    tx: &mut PgConnection,
    project_id: ProjectId,
    created_by: UserId,
) -> Result<()> {
    // For the row constructors only; every timestamp column takes the server's clock.
    let now = Utc::now();

    for (position, kind) in seed::KINDS.iter().enumerate() {
        let graph = seed::graph_row(StepGraphId::new(), project_id, kind, now);
        sqlx::query!(
            "INSERT INTO step_graph (id, project_id, name, description) VALUES ($1, $2, $3, $4)",
            graph.id.as_uuid(),
            graph.project_id.as_uuid(),
            graph.name,
            graph.description,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        for (phase_position, phase) in kind.phases.iter().enumerate() {
            let row = seed::phase_row(PhaseId::new(), graph.id, phase_position as i32, phase, now);
            sqlx::query!(
                "INSERT INTO step_graph_phase (id, graph_id, position, name, fan_out, gate, \
                 gate_hard, retry_limit, input_kinds, output_kind, isolation, command_queue, \
                 verify_command, template_name, template_version, token_budget) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
                row.id.as_uuid(),
                row.graph_id.as_uuid(),
                row.position,
                row.name,
                row.fan_out,
                row.gate.as_str(),
                row.gate_hard,
                row.retry_limit,
                &row.input_kinds[..],
                row.output_kind,
                row.isolation.map(Isolation::as_str),
                row.command_queue.as_str(),
                row.verify_command.as_deref(),
                row.template_name,
                row.template_version,
                row.token_budget,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        let kind_row = seed::kind_row(
            ItemKindId::new(),
            project_id,
            graph.id,
            position as i32,
            kind,
            now,
        );
        sqlx::query!(
            "INSERT INTO item_kind (id, project_id, prefix, name, description, default_graph_id, \
             position) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            kind_row.id.as_uuid(),
            kind_row.project_id.as_uuid(),
            kind_row.prefix,
            kind_row.name,
            kind_row.description,
            kind_row.default_graph_id.as_uuid(),
            kind_row.position,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    for (name, _, body) in &DEFAULT_TEMPLATES {
        let row = seed::template_row(
            PromptTemplateId::new(),
            project_id,
            name,
            body,
            created_by,
            now,
        );
        sqlx::query!(
            "INSERT INTO prompt_template (id, project_id, name, version, body, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            row.id.as_uuid(),
            row.project_id.as_uuid(),
            row.name,
            row.version,
            row.body,
            row.created_by.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    Ok(())
}

/// The refusal a delete gets after [`DELETE_ATTEMPTS`] serialization failures (review M1).
///
/// It says what happened and that nothing happened, because the alternative a caller might assume —
/// a partial delete — is the one thing a transaction cannot leave behind.
fn concurrent_write(entity: &str, id: impl core::fmt::Display, what: &str) -> String {
    format!(
        "{entity} {id} is being written to: {DELETE_ATTEMPTS} attempts each found {what} committed \
         after the count, and the delete took nothing"
    )
}

/// MOD-12 M2 D9: the reads behind [`WriteStore::queue_setting`] and the queue writes' misses.
impl PgStore {
    /// One queue key with its token, the target check already made. `App` and `Project` are
    /// `stored_setting`'s statements byte for byte, so they reuse its offline entries.
    async fn stored_queue_setting(
        &self,
        target: QueueTarget,
        key: QueueSetting,
    ) -> Result<Option<QueueStored>> {
        let name = key.as_str();
        match target {
            QueueTarget::App => {
                let row = sqlx::query!(
                    "SELECT value, updated_at FROM app_setting WHERE key = $1",
                    name,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                Ok(row.map(|row| QueueStored {
                    value: Some(row.value),
                    token: QueueToken::Stamp(Some(row.updated_at)),
                }))
            }
            QueueTarget::Project(id) => {
                let row = sqlx::query!(
                    "SELECT settings, updated_at FROM project WHERE id = $1",
                    id.as_uuid(),
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                Ok(row.map(|row| QueueStored {
                    value: row.settings.get(name).cloned(),
                    token: QueueToken::Stamp(Some(row.updated_at)),
                }))
            }
            QueueTarget::Box(id) => {
                let me = self.this_user();
                Ok(self
                    .box_row(id)
                    .await?
                    .filter(|row| row.user_id == me)
                    .map(|row| QueueStored {
                        value: row.settings.get(name).cloned(),
                        token: QueueToken::EditVersion(row.edit_version),
                    }))
            }
        }
    }

    /// A box queue write that touched no row, decided as `edit_box`'s miss is: no box of this
    /// user is `NotFound`; the current token over a non-object blob is
    /// [`BOX_SETTINGS_NOT_AN_OBJECT`]; anything else is `Stale` with the key as stored now.
    async fn box_queue_miss(
        &self,
        id: BoxId,
        expected: i32,
        key: QueueSetting,
    ) -> Result<CasOutcome<QueueStored>> {
        let me = self.this_user();
        match self.box_row(id).await?.filter(|row| row.user_id == me) {
            None => Err(StoreError::NotFound {
                entity: "box",
                id: id.to_string(),
            }),
            Some(row) if row.edit_version == expected && !row.settings.is_object() => Err(
                StoreError::Constraint(BOX_SETTINGS_NOT_AN_OBJECT.to_owned()),
            ),
            Some(row) => Ok(CasOutcome::Stale(QueueStored {
                value: row.settings.get(key.as_str()).cloned(),
                token: QueueToken::EditVersion(row.edit_version),
            })),
        }
    }
}

impl PgStore {
    /// One `agent` row by id, for [`WriteStore::upsert_agent`]'s miss (MOD-40 blueprint B12).
    ///
    /// `agents()` is the registry joined to this box and answers every row; the compare-and-set
    /// needs exactly the one it missed, as stored now.
    async fn stored_agent(&self, id: AgentId) -> Result<Option<Agent>> {
        sqlx::query_as!(
            Agent,
            r#"
            SELECT id            AS "id: AgentId",
                   name,
                   transport     AS "transport: htui_core::model::Transport",
                   launch,
                   models,
                   default_model,
                   billing       AS "billing: htui_core::model::Billing",
                   enabled,
                   settings,
                   created_at,
                   updated_at
              FROM agent
             WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// One attempt of [`WriteStore::delete_workspace`]'s count-and-delete; `Ok(None)` is the
    /// `40001` the caller retries (review M1).
    async fn delete_workspace_once(&self, id: WorkspaceId) -> Result<Option<DeleteReach>> {
        let mut tx = begin_repeatable_read(&self.pool).await?;

        let Some(reach) = workspace_reach(&mut tx, id).await? else {
            return Err(StoreError::NotFound {
                entity: "workspace",
                id: id.to_string(),
            });
        };
        match sqlx::query!("DELETE FROM workspace WHERE id = $1", id.as_uuid())
            .execute(&mut *tx)
            .await
        {
            Ok(_) => {}
            Err(err) if is_serialization_failure(&err) => return Ok(None),
            Err(err) => return Err(map_sqlx(err)),
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(Some(reach))
    }

    /// One attempt of [`WriteStore::delete_project`]'s count-and-delete; `Ok(None)` is the `40001`
    /// the caller retries (review M1).
    ///
    /// The count is the transaction's first statement after the isolation level is set, so it is
    /// the statement that takes the snapshot the cascade is then crosschecked against: only the
    /// `DELETE` can raise the serialization failure, which is why only it is matched on.
    async fn delete_project_once(&self, id: ProjectId) -> Result<Option<DeleteReach>> {
        let mut tx = begin_repeatable_read(&self.pool).await?;

        let Some(reach) = project_reach(&mut tx, id).await? else {
            return Err(StoreError::NotFound {
                entity: "project",
                id: id.to_string(),
            });
        };
        release_repo_references(&mut tx, id).await?;
        match sqlx::query!("DELETE FROM project WHERE id = $1", id.as_uuid())
            .execute(&mut *tx)
            .await
        {
            Ok(_) => {}
            Err(err) if is_serialization_failure(&err) => return Ok(None),
            Err(err) => return Err(map_sqlx(err)),
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(Some(reach))
    }

    /// Which half of the `item_kind` writers' folded `EXISTS` guard refused, in the sentence
    /// `MemStore` uses for it (D11).
    ///
    /// The guard is one conjunct and covers two rules — an unknown project and a graph that is not
    /// the project's — so the split is made here, on the failure path only, rather than by two
    /// statements on every create.
    async fn kind_guard_refusal(&self, project: ProjectId, graph: StepGraphId) -> StoreError {
        match self.project(project).await {
            Ok(Some(_)) => StoreError::Constraint(graph_not_in_project(graph, project)),
            Ok(None) => StoreError::Constraint(format!(
                "item_kind.project_id `{project}` references no project"
            )),
            Err(err) => err,
        }
    }

    /// One `item_revision` row: the ancestor a [`UpdateOutcome::Diverged`] answer is rendered
    /// against (ANA-9 §4.2).
    ///
    /// Private because §6.1 exposes revisions only through the divergence answer; MOD-11's history
    /// view will promote it.
    async fn revision(&self, id: ItemId, version: i32) -> Result<ItemRevision> {
        sqlx::query_as!(
            ItemRevision,
            r#"
            SELECT item_id   AS "item_id: ItemId",
                   version,
                   title,
                   body,
                   required_tags,
                   author_id AS "author_id: htui_core::model::UserId",
                   box_id    AS "box_id: htui_core::model::BoxId",
                   reason,
                   created_at
              FROM item_revision WHERE item_id = $1 AND version = $2
            "#,
            id.as_uuid(),
            version,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item_revision",
            id: format!("{id}@{version}"),
        })
    }
}

/// MOD-12 M1: the queue's writes (plan D1-D3, D9). Neither `queue_entry` nor `queue_batch` is
/// mirrored, so these are inherent and `Backend` dispatches them; `MemStore` carries the same.
impl PgStore {
    /// MOD-12 D1: queue `item` on `box_id`. Idempotent: an item already queued (on any box)
    /// answers its stored entry unchanged (`ON CONFLICT (item_id) DO NOTHING`, then the read).
    /// `at` is microsecond-truncated by the driver.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] `{ entity: "item" }` for an unknown item;
    /// [`StoreError::Constraint`] for an unknown box or user (23503, through [`map_sqlx`]).
    pub async fn queue_item(
        &self,
        item: ItemId,
        box_id: BoxId,
        by: UserId,
        at: DateTime<Utc>,
    ) -> Result<QueueEntry> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query_scalar!(
            r#"SELECT 1 AS "one!" FROM item WHERE id = $1"#,
            item.as_uuid()
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: item.to_string(),
        })?;
        sqlx::query!(
            r#"
            INSERT INTO queue_entry (item_id, box_id, position, queued_at, queued_by)
            VALUES ($1, $2, NULL, $3, $4)
            ON CONFLICT (item_id) DO NOTHING
            "#,
            item.as_uuid(),
            box_id.as_uuid(),
            at,
            by.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let entry = sqlx::query_as!(
            QueueEntry,
            r#"
            SELECT e.item_id    AS "item_id: ItemId",
                   i.project_id AS "project_id: ProjectId",
                   e.box_id     AS "box_id: BoxId",
                   e.position,
                   e.queued_at,
                   e.queued_by  AS "queued_by: UserId"
              FROM queue_entry e JOIN item i ON i.id = e.item_id
             WHERE e.item_id = $1
            "#,
            item.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(entry)
    }

    /// MOD-12 D9: `item` leaves whatever queue holds it; `false` when none did. Never touches a
    /// run.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn dequeue_item(&self, item: ItemId) -> Result<bool> {
        let gone = sqlx::query!("DELETE FROM queue_entry WHERE item_id = $1", item.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
        Ok(gone == 1)
    }

    /// MOD-12 M3 D3: moves `item` one place `to` in `box_id`'s queue, atomically. The first move
    /// of a queue writes `position = 1..n` over every entry in the current D2 order, so an entry
    /// queued later (`NULL`) goes after them. `false`, writing nothing, when `item` is not in
    /// `box_id`'s queue or is already at that end. Never touches `item.priority`.
    ///
    /// One transaction, three statements (blueprint §B.3.3, §F-12):
    /// 1. lock the box's entries in `item_id` order (review R1 L1), the order
    ///    [`prune_finished_entries`](PgStore::prune_finished_entries) locks in too, so a move and
    ///    a prune never wait on each other in a cycle. The rows it answers are only the locked
    ///    set: under `READ COMMITTED` a locking select that waited on a concurrent move may
    ///    answer pre-wait values, so the order is not read here;
    /// 2. re-read them in D2 order with [`queue_entries`](PgStore::queue_entries)' literal, byte
    ///    for byte; a later statement, its snapshot sees every commit the lock waited for. An
    ///    entry the lock did not take (queued since) is left out, so the write below never waits
    ///    on a row lock out of order;
    /// 3. write the moved order as `position = 1..n` in one `UNNEST` update.
    ///
    /// A concurrent `queue_item` inserts `NULL`, which sorts after the written positions; a
    /// concurrent `dequeue_item` waits on the row lock.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn move_queue_entry(
        &self,
        box_id: BoxId,
        item: ItemId,
        to: QueueMove,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let locked: BTreeSet<Uuid> = sqlx::query_scalar!(
            "SELECT item_id FROM queue_entry WHERE box_id = $1 ORDER BY item_id FOR UPDATE",
            box_id.as_uuid(),
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .into_iter()
        .collect();
        let entries = sqlx::query_as!(
            QueueEntry,
            r#"
            SELECT e.item_id    AS "item_id: ItemId",
                   i.project_id AS "project_id: ProjectId",
                   e.box_id     AS "box_id: BoxId",
                   e.position,
                   e.queued_at,
                   e.queued_by  AS "queued_by: UserId"
              FROM queue_entry e JOIN item i ON i.id = e.item_id
             WHERE e.box_id = $1
             ORDER BY e.position NULLS LAST, i.priority DESC, i.created_at, i.id
            "#,
            box_id.as_uuid(),
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let order: Vec<ItemId> = entries
            .iter()
            .map(|entry| entry.item_id)
            .filter(|id| locked.contains(&id.as_uuid()))
            .collect();
        let Some(moved) = moved_order(&order, item, to) else {
            return Ok(false); // the dropped `tx` rolls back: nothing was written
        };
        let items: Vec<Uuid> = moved.iter().map(|id| id.as_uuid()).collect();
        let positions: Vec<i32> = (1..).take(items.len()).collect();
        sqlx::query!(
            r#"
            UPDATE queue_entry e
               SET position = v.position
              FROM UNNEST($2::uuid[], $3::int4[]) AS v(item_id, position)
             WHERE e.box_id = $1 AND e.item_id = v.item_id
            "#,
            box_id.as_uuid(),
            &items[..],
            &positions[..],
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(true)
    }

    /// MOD-12 D2: resume — the open batch of `box_id`, opened now under a fresh [`BatchId`]
    /// unless one is already open. Idempotent: `uq_queue_batch_open` turns the second of two
    /// racing inserts into a no-op, and both answer the row that won. A close that slips between
    /// the insert and the read is retried once.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for an unknown box or user (23503, through [`map_sqlx`]);
    /// [`StoreError::Backend`] when no batch is open after the retry.
    pub async fn open_batch(
        &self,
        box_id: BoxId,
        by: UserId,
        at: DateTime<Utc>,
    ) -> Result<QueueBatch> {
        for _ in 0..2 {
            sqlx::query!(
                r#"
                INSERT INTO queue_batch (id, box_id, opened_at, opened_by)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (box_id) WHERE closed_at IS NULL DO NOTHING
                "#,
                BatchId::new().as_uuid(),
                box_id.as_uuid(),
                at,
                by.as_uuid(),
            )
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
            if let Some(open) = self.open_batch_of(box_id).await? {
                return Ok(open);
            }
        }
        Err(StoreError::Backend(
            "queue_batch: no open batch after open".to_owned(),
        ))
    }

    /// MOD-12 D2, D3: close `box_id`'s open batch with `reason`; `None` when none was open. The
    /// UPDATE conflicts with `create_run`'s `FOR SHARE` on the row, so a pause and an admission
    /// serialise (H-6).
    ///
    /// In the same transaction, every run of the batch still `queued` is cancelled through
    /// [`finish_run_on`], so its item goes back to `open` by `finish_run`'s mirror and its queue
    /// entry stays (review H2): a pause stops the runs no process has claimed yet, not only the
    /// admissions. The runs are locked `FOR UPDATE` after the batch row, so a racing `claim_run`
    /// (which locks the run row first) either claims the run before this reads it - the run is
    /// `running` and left alone - or finds it `cancelled` and answers `NotClaimable`. A `drained`
    /// close finds no such run: the drain closes only a batch whose runs are all terminal.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn close_batch(
        &self,
        box_id: BoxId,
        reason: BatchClose,
        at: DateTime<Utc>,
    ) -> Result<Option<QueueBatch>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let closed = sqlx::query_as!(
            QueueBatch,
            r#"
            UPDATE queue_batch
               SET closed_at = $3, closed_reason = $2
             WHERE box_id = $1 AND closed_at IS NULL
            RETURNING id AS "id: BatchId", box_id AS "box_id: BoxId", opened_at,
                      opened_by AS "opened_by: UserId", closed_at,
                      closed_reason AS "closed_reason: BatchClose"
            "#,
            box_id.as_uuid(),
            reason.as_str(),
            at,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if let Some(batch) = &closed {
            let waiting = sqlx::query_scalar!(
                r#"
                SELECT id AS "id: RunId"
                  FROM run
                 WHERE batch_id = $1 AND status = 'queued'
                 ORDER BY queued_at, id
                   FOR UPDATE
                "#,
                batch.id.as_uuid(),
            )
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            for run in waiting {
                finish_run_on(&mut tx, run, RunStatus::Cancelled, None, at).await?;
            }
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(closed)
    }

    /// MOD-12 D3 (review M1), M3 D4 (L4): the runner's close of exactly `batch`, only while it is
    /// open and no run of its own is `queued`, `running` or `awaiting_approval`. The entries the
    /// runner read do not keep it open: it calls this when nothing is admissible (an empty queue,
    /// or a stalled one). An entry it did not read does (review R1 L2): `seen` is the items of
    /// the entries it read (empty when it read none), and an entry of the box for any other item
    /// means an item may be ready that the runner never judged. The check is by item, not by
    /// `queued_at`: that is the queuing client's clock, not commit order, so a skewed clock or a
    /// queuer that stamped first and committed last could date an unread entry before every
    /// entry read. The re-check is the UPDATE's own `WHERE`, so a resume that opened a new batch
    /// after the runner's reads is never closed by it, and a run or an entry that committed before
    /// the statement keeps the batch open. `None` when the batch did not close.
    ///
    /// An admission in flight (`create_run` past its `FOR SHARE` on the batch, not yet committed)
    /// is waited for first, by a `FOR UPDATE` on the batch row in a statement of its own. Without
    /// it the UPDATE would wait on the share lock and then, the row being only locked and not
    /// updated, go ahead on its statement-start snapshot, which never sees the admitted run; with
    /// no entry clause left to cover that window (L4) the batch would close over a `queued` run.
    /// Under `READ COMMITTED` the UPDATE, a later statement, sees every admission that held the
    /// share lock, and none can take it again until this transaction ends.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn close_drained_batch(
        &self,
        batch: BatchId,
        seen: &[ItemId],
        at: DateTime<Utc>,
    ) -> Result<Option<QueueBatch>> {
        let seen: Vec<Uuid> = seen.iter().map(|item| item.as_uuid()).collect();
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let open = sqlx::query_scalar!(
            r#"SELECT 1 AS "one!" FROM queue_batch WHERE id = $1 AND closed_at IS NULL FOR UPDATE"#,
            batch.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if open.is_none() {
            return Ok(None);
        }
        let closed = sqlx::query_as!(
            QueueBatch,
            r#"
            UPDATE queue_batch b
               SET closed_at = $2, closed_reason = 'drained'
             WHERE b.id = $1 AND b.closed_at IS NULL
               AND NOT EXISTS (
                       SELECT 1 FROM run r
                        WHERE r.batch_id = b.id
                          AND r.status IN ('queued', 'running', 'awaiting_approval'))
               AND NOT EXISTS (
                       SELECT 1 FROM queue_entry e
                        WHERE e.box_id = b.box_id
                          AND e.item_id <> ALL($3::uuid[]))
            RETURNING b.id AS "id: BatchId", b.box_id AS "box_id: BoxId", b.opened_at,
                      b.opened_by AS "opened_by: UserId", b.closed_at,
                      b.closed_reason AS "closed_reason: BatchClose"
            "#,
            batch.as_uuid(),
            at,
            &seen,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(closed)
    }

    /// MOD-12 D3: drop `box_id`'s entries whose item is `done` or `closed`; how many went.
    ///
    /// The rows are locked in `item_id` order first (review R1 L1), the order
    /// [`move_queue_entry`](PgStore::move_queue_entry) locks in, so the runner's prune and a
    /// user's move never deadlock. A bare `DELETE … USING item` would lock in whatever order its
    /// plan visits the rows.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn prune_finished_entries(&self, box_id: BoxId) -> Result<u64> {
        Ok(sqlx::query!(
            r#"
            WITH doomed AS (
                SELECT e.item_id
                  FROM queue_entry e JOIN item i ON i.id = e.item_id
                 WHERE e.box_id = $1 AND i.status IN ('done', 'closed')
                 ORDER BY e.item_id
                   FOR UPDATE OF e)
            DELETE FROM queue_entry e
             USING doomed d
             WHERE e.item_id = d.item_id
            "#,
            box_id.as_uuid(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected())
    }
}
