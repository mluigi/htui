# Blueprint: MOD-40, multi-writer store hardening

**Status**: proposed (2026-09-26). Milestone 1 only. Findings F-1 to F-12 (§0) and decisions
B1–B9 (§0a) are this blueprint's. A **Blocker** means the plan, read literally, fails an existing
test, does not compile at a task boundary, or cannot pin its own claim. The Fix column is what the
implementer builds.

**Plan**: `.claude/plans/mod-40-multi-writer-hardening.plan.md`, CONFIRMED 2026-09-26, OQ-1 and
OQ-2 as recommended. Its D1–D3 and T1–T2 are the scope here, amended only where §0 says so.
**PRD**: `.claude/prds/mod-40-multi-writer-hardening.prd.md`; PRD D1 wins over this blueprint.

**Verified at**: `c0ff8f5` (branch `claude/project-thread-bqiyzj`); `crates/` is unchanged since
`edf19c0`, so the plan's citations hold. Paths are relative to `crates/`. **Line numbers are
pre-edit.** Every fact was read with `grep`/`sed`. `graphify-out/` does not exist and Gortex is not
reachable. Nothing here comes from a code graph.

**Probes** (Postgres 16.13, `localhost:5432`, scratch database `mod40_probe` built from migrations
`0001`–`0007`, dropped afterwards; a scratch crate for the MemStore probe, in the scratchpad):
- P-1. The §2.4 `append_events` statement, cases A–K below: all as specified.
- P-2. `IS NOT DISTINCT FROM $n` with a `NULL` uuid: matches a `NULL` `lease_owner` and nothing
  else. An **untyped** `PREPARE` (what sqlx's describe does) infers `{jsonb,uuid}` for the append
  (result `{bigint,uuid}`), `{uuid,jsonb,text,uuid}` for `set_step_usage` and
  `{uuid,integer,jsonb,jsonb,text,integer,timestamptz,uuid}` for `finish_step`.
- P-3. `FOR SHARE` inside the `EXISTS` of an `UPDATE`, and `FOR SHARE OF r` in a CTE: accepted.
  A write racing an uncommitted `UPDATE run SET lease_owner = B` **waited 2.0 s** for it and then
  answered `fenced_step = <step>`; the same statement without the lock did not wait.
- P-4. The plan's literal shape (`… JOIN run_step … JOIN run … WHERE r.lease_owner IS NOT
  DISTINCT FROM $2`) on a batch `[leased step seq 7, missing step seq 0]`: `INSERT 0 1` (F-1).
- P-5. `MemStore::demo()`: `claim_run(ids::RUN_2, ids::BOX, owner, at, at + 5 min)` →
  `Ok(Admitted)`; `STEP_R2_PRD` is its one step, `Pending`.

**Scope**: no migration; `TABLES`, migration pins and commented-column pins do not move. One new
type (`StepFence`), one new `StoreError` variant, three `WriteStore` signatures across five
implementors. `.sqlx/` stays at **281** files (3 replaced). Store `CASES` 83 → **89**; orch
`CASES` 72 → **73**; `READ_CASES` stays 14.

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs`, `missing_debug_implementations`,
`unused_qualifications` warn and clippy runs `-D warnings`; rustdoc denies broken/private
intra-doc links. `rustfmt.toml` `max_width = 100`. Every commit compiles; red first, then green.
`every_cross_referenced_test_name_exists` (`htui-core/src/store/conformance.rs:11465`) fails on a
backticked snake_case name of ≥4 underscores that `conformance.rs`/`mem.rs` do not define: do
**not** backtick `a_suspended_walk_cannot_write_after_adoption` or any `htui-agent` test name in
`htui-core`.

---

## 0. Findings

| # | Severity | Plan says | Tree / probe | Fix |
|---|---|---|---|---|
| **F-1** | **Blocker** (fails an existing case) | D1: `INSERT … SELECT … FROM jsonb_to_recordset($1) e JOIN run_step s … JOIN run r … WHERE r.lease_owner IS NOT DISTINCT FROM $2`; a miss told apart by a follow-up read. | The join **drops** rows for a missing step instead of failing the statement, so a mixed batch writes its good rows (P-4: `INSERT 0 1`). `append_events_idempotent_and_ordered` asserts the opposite (`conformance.rs:1356-1373`: `[STEP_IMPL seq 7, orphan seq 0]` → `Constraint` and `after.len() == 7`), and so does the trait (`traits.rs:288-289`). | B1: a CTE computes the fence verdict and inserts **all or nothing**; a missing step still reaches the `INSERT` and fails on `23503` (§2.4, P-1). |
| **F-2** | Major | D1: `append_events` runs the follow-up read "only when fewer rows landed than were offered". | A follow-up read after the insert is a second snapshot (TOCTOU), and it is not needed: the verdict is a column of the same statement. | B1: the statement returns `(inserted, fenced_step)`; no follow-up read for the append. The two `UPDATE`s keep the plan's follow-up read, via the existing `step_exists` (`pg/write.rs:168-178`, `interrupt_step`'s shape `:4124-4127`), so they add **no** `.sqlx` file. |
| **F-3** | Major (the fence is not a fencing token without it) | D1: "the predicate joins through `run` in the same statement". | Under `READ COMMITTED` a join or `EXISTS` **reads** `run`; it does not lock it. A write whose snapshot predates a `take_lease`/`adopt_runs` commit can commit after it. P-3: with `FOR SHARE` the write waited for the uncommitted take and answered fenced. | B2: `FOR SHARE OF r` in the append's `lease` CTE and `FOR SHARE` in the two `EXISTS`. Either the write locks first (the take waits: the write is linearised before it) or the take does (the write re-reads and is fenced). Pinned by the Postgres-only case §2.9. |
| **F-4** | **Blocker** (T1 cannot pass its own gate) | T1 = store only; D2 (recorder fence, engine) is T2. T1 validates with `cargo build --workspace --all-targets`. | `record.rs:1042`, `:1114` and the five engine sites call the changed methods, so the workspace does not build after T1. Updating the engine's `finish_step` to `Lease(owner)` in T1 without the recorder fence fails **every** orch walk case: the recorder's default `Unleased` meets a claimed run and is `Fenced` at `record_prompt`. | B3: **D2's plumbing moves into T1** (fence field, `with_fence`, the two recorder call sites, `open_recorder`, the five `finish_step` sites). T2 keeps D3, its tests and the orch case. |
| **F-5** | Major (D3 misses a collision) | D3: `flush` "first re-offers `unflushed` alone (a short count is fine: that is the replay)". | `release_held` pushes **never-offered** `edit_proposal` rows into `unflushed` (`record.rs:1102`, doc `:391-393`). Under D3 as written a collision on a held row's reserved `seq` would pass as a replay. | B4: a second vector, `unoffered`, for numbered rows never offered (released held rows, and a batch numbered by a flush whose replay was refused). They go in the **fresh** call (§3.2). |
| **F-6** | Major (the named orch case cannot observe what it names) | T2: `a_suspended_walk_cannot_write_after_adoption` — "A's recorder flush and `finish_step` are `Fenced`". | The only suspend seam is `stall_after_done`, which **never returns** (`fake.rs:1616-1621`), and it sits after the session: `session` finishes the recorder (`engine.rs:5257`) before `after_done` (`:3152-3156`). No seam pauses a walk mid-session. | B5: add a wakeable `suspend_after_done` (fake + `Orchestrate`). The case pins the engine's own `finish_step` after adoption, and a recorder built exactly as `open_recorder` builds it. The engine's recorder wiring is pinned by the whole orch suite: a missing `.with_fence` answers `Fenced` at `record_prompt` on every claimed run. |
| **F-7** | Minor (record) | OQ-1: "a stale holder that wakes mid-session meets a fenced recorder flush first and its walk stops there". | A holder that wakes **after `done`** writes `run_step_commit.after_hash` (`record_commits`, `engine.rs:3174`, `:3773`, `:1398`, `:4704`) and, in production, the sink's output document before it reaches the fenced `finish_step`. | Out of scope (OQ-1). The orch case asserts the `run_step` row and the log only. Add to the MOD-41 note: `record_commits` and the output write are unfenced step writes on the settle tail. |
| **F-8** | Minor (file sets) | T1 lists `htui/tests/chat_usage_pg.rs`; T2 lists neither the test doubles nor the fake. | `chat_usage_pg.rs` has no call site (only `Recorder::new`, `:175`, `:259`: default fence). T2 needs `htui-agent/tests/recorder.rs` (SpyStore knobs), `htui-orch/src/fake.rs` (B5) and `htui-orch/tests/fake_conformance.rs:16` (pin). | §5's file lists. |
| **F-9** | Minor (tooling) | "`cargo sqlx prepare` for the three changed queries." | `cargo sqlx` is not installed here; `Cargo.lock` pins `sqlx 0.9.0`. README's DSN (`postgres:htui@…:5439`, `README.md:501-505`) is not this box's. | §6: `sqlx-cli 0.9.0`, `postgres://htui:htui@localhost:5432/htui_sqlx`. |
| **F-10** | Minor (docs) | D3: fix the "offline buffer" text at `traits.rs:285-295`, `write.rs:937-939`, `writer.rs:15-24`, `:85-106`. | Also stale: `traits.rs:258-264` (the trait head names the offline sink), `writer.rs:8-10` (`BufferedWriter`), `write.rs:8-9` and `:933-934` (`crate::cache::pending` no longer exists: `htui-store/src/cache/` holds `mod.rs`, `read.rs`, `refresh.rs`), `record.rs:240-245` ("milestone 4's offline sink"; "`seq` did not advance", wrong since plan D77, `:1010-1014`), `record.rs:396-397` (`cache/pending.rs`). `writer.rs:82-113` is the orphaned doc of the deleted `BufferedWriter`, now glued onto `REGISTRY_ON_SERVER_ONLY`'s doc (`:114-120`). | §2.3, §2.5, §2.6, §3.2 give each replacement. |
| **F-11** | Minor (accepted, R-1) | R-1: a chat continuing a promoted step meets a lease only if accept takes it while the chat is live. | Also: a park whose best-effort `release_lease` failed (`engine.rs:1877-1890`, warned) leaves `lease_owner` set; a chat on that promoted step is then `Fenced` until the owner's sweep gives the lease back (`DeadWalks`, plan D140). Loud, never silent. | None. Add one sentence to R-1. |
| **F-12** | Minor | — | `record_unreadable`'s `(Err, Some(breach))` arm logs "the row is written with the closing pair" (`record.rs:1473-1476`); after B4 a collision's rows are **not** re-offered. | Change the text to "the flush of a breaching row failed; the cap is enforced anyway (blueprint H-2)". |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B1** (F-1, F-2): the append's fence verdict is computed and returned by the statement itself;
  the insert is all-or-nothing on the fence; a missing step still fails on its FK.
- **B2** (F-3): the fenced statements share-lock the `run` row.
- **B3** (F-4): D2's plumbing is in T1.
- **B4** (F-5): `unoffered` beside `unflushed`.
- **B5** (F-6): `suspend_after_done` on the fake and on `Orchestrate`.
- **B6**: precedence, identical on both stores: missing step (`Constraint` for the append,
  `NotFound` for the updates) **before** `Fenced` **before** a `CHECK` violation. For a batch naming
  several fenced steps, `Fenced { step }` names the lowest `StepId` (`Ord` is bytewise on both
  sides: Postgres `uuid` ordering, `StepId: Ord`, `model/ids.rs:19-22`).
- **B7**: a fence miss is answered even when every row of the batch is already stored (a stale
  holder's replay is `Fenced`, never `Ok(0)`).
- **B8**: the three recorder tests live in `htui-agent/tests/recorder.rs`, not `record.rs`: two
  need `SpyStore`'s call log and a lost-reply knob, and all three read better beside
  `a_refused_append_costs_a_retry_and_never_a_seq` (`:3208`).
- **B9**: the orch case lands in T2 but passes as soon as T1 is in; it is a pin, and T2's red
  commit is the three recorder tests.

---

## Milestone 1 — step fence (T1, T2)

### 1. Build order

| Task | Crates | Commits (each compiles) | Gate |
|---|---|---|---|
| T1 | htui-core, htui-store, htui-agent, htui-orch, htui (tests) | (1) red: types, signatures, every call site and impl threading `fence` **without enforcing it**, the six store cases and the Postgres race case (fail); (2) green: MemStore and PgStore enforce, `.sqlx` regenerated, docs. | §6 T1 |
| T2 | htui-agent, htui-orch | (1) red: SpyStore knobs, the three recorder tests (two fail), fake `suspend_after_done`, the orch case (passes, B9); (2) green: the `flush` rewrite, `unoffered`, docs. | §6 T2 |

### 2. T1 — the fence in the store (D1, D2 plumbing)

#### 2.1 `htui-core/src/store/traits.rs` — `StepFence`

Insert after `impl<T> CasOutcome<T>` (ends `:1989`), before `DeleteTarget`'s doc (`:1991`).
`Uuid` is already imported (`:33`).

```rust
/// Which lease a step write is made under (MOD-40 plan D1, PRD D1).
///
/// [`WriteStore::append_events`], [`WriteStore::set_step_usage`] and [`WriteStore::finish_step`]
/// take one and write only while the step's run carries exactly that lease:
/// `run.lease_owner IS NOT DISTINCT FROM` [`StepFence::owner`]. A process whose run another process
/// adopted ([`WriteStore::adopt_runs`], [`WriteStore::take_lease`]) still holds its old `Lease`,
/// and the store answers it with [`StoreError::Fenced`](crate::store::StoreError::Fenced) and
/// writes nothing.
///
/// No `Default`: every caller says which one it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepFence {
    /// The walk's own lease: the `owner` it passed to `claim_run`, `take_lease` or `adopt_runs`
    /// (the engine's `parts.owner`). Writes only while `run.lease_owner = owner`.
    Lease(Uuid),
    /// No lease: a chat run, whose `lease_owner` is `NULL`, or a promoted step continued by a chat
    /// after its park released the lease. Refused on a run whose lease names an owner.
    Unleased,
}

impl StepFence {
    /// The `run.lease_owner` this fence writes under: the owner, or `None` for `NULL`.
    #[must_use]
    pub const fn owner(self) -> Option<Uuid> {
        match self {
            Self::Lease(owner) => Some(owner),
            Self::Unleased => None,
        }
    }
}
```

`htui-core/src/store/mod.rs:15-17`: add `StepFence` to the `pub use traits::{…}` list after
`SettingRung` (then `cargo fmt`).

#### 2.2 `htui-core/src/store/error.rs` — `StoreError::Fenced`

`:3` becomes `use crate::model::{ParseEnumError, StepId};`. Insert after `Constraint` (`:21`):

```rust
    /// A step write named a lease its run does not carry (MOD-40 plan D1): another process
    /// adopted the run, or the fence names a lease on a run that has none, or none on a run that
    /// has one. Nothing was written, and a retry under the same fence answers the same: the writer
    /// has lost the step.
    #[error("run_step {step} is not writable under this lease")]
    Fenced {
        /// The step the write named; for a batch, the lowest-ordered fenced one.
        step: StepId,
    },
```

**No exhaustive match breaks.** An exhaustive `match` on `StoreError` must name every variant or
use `_`; `grep -rn 'ReadOnly(\|ParseEnum(' crates --include=*.rs` finds only the definitions
(`error.rs:25`, `:41`), so every `match` on `StoreError`, production or test, already has a
wildcard. Known single-arm sites stay as they are: `connection.rs:205-208`, `hierarchy.rs:638`,
`store_worker.rs:1973`, `record.rs:1183-1190`. `StepId` is `Clone + Eq + Display`, so the
enum's derives (`Debug, Clone, PartialEq, Eq, thiserror::Error`, `:9`) hold.

#### 2.3 Trait signatures and docs (`traits.rs`)

`:256-264`, current:

```text
/// Everything a write path needs.
///
/// It used to be implemented only by a store that can reach Postgres. Since MOD-2 milestone 4
/// (plan D34) `htui-store`'s offline sink implements it too, writing the `session_event` rows to
/// a JSON-lines buffer and answering
/// [`StoreError::Unreachable`](crate::store::StoreError::Unreachable) for everything the buffer
/// cannot hold — so "an offline write is a compile error" is now narrower and still true where it
/// counts: nothing reaches **Postgres** except through a store that has a connection, and the
/// read-only mirror still does not implement this trait at all.
```

replacement:

```rust
/// Everything a write path needs.
///
/// Implemented by the stores that write where they read — `PgStore`, which needs a connection,
/// and `MemStore` — and by `htui-store`'s `Writer`, which holds one of the two. The read-only
/// mirror does not implement it, so an offline write is a compile error. (Between MOD-2 milestone
/// 4 and MOD-25 an offline sink implemented it too; MOD-25 removed it.)
```

`:285-296` becomes:

```rust
    /// Appends session events under `fence`, skipping any `(run_step_id, seq)` already stored, and
    /// answers how many rows were actually inserted (`docs/ANA-4.md` §4.1, `docs/ANA-9.md` §4.3,
    /// MOD-40 plan D1).
    ///
    /// One statement: either every new row lands or none does, so a batch naming a step that does
    /// not exist, or a step whose run does not carry `fence`'s lease, writes nothing at all.
    /// Idempotence is the primary key's: a row already stored is skipped and not counted. That is
    /// what makes the recorder's re-offer of a batch whose answer it never got safe, and why such a
    /// replay may answer less than it offered; a **fresh** batch that answers less has met a second
    /// writer, which the recorder reports (MOD-40 plan D3).
    ///
    /// # Errors
    ///
    /// In this order: [`StoreError::Constraint`](crate::store::StoreError::Constraint) when an
    /// event names a `run_step` that does not exist;
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced) when a named step's run has a
    /// `lease_owner` other than `fence`'s, even if every row is already stored;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `kind` / `role`
    /// outside the §4.3 `CHECK` lists.
    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> Result<usize>;
```

`:298-312` keeps its first two paragraphs and becomes:

```rust
    /// …(two paragraphs unchanged)…
    ///
    /// Written only while the step's run carries `fence`'s lease (MOD-40 plan D1).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) when the step does not exist;
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced) when it does and its run's
    /// `lease_owner` is not `fence`'s.
    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()>;
```

`:1078-1082` becomes:

```rust
    /// Writes the settle columns of [`StepOutcome`]; never `status`. Only while the step's run
    /// carries `fence`'s lease (MOD-40 plan D1).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`;
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced) when the step exists and its run's
    /// `lease_owner` is not `fence`'s.
    async fn finish_step(&self, fence: StepFence, step: StepId, outcome: StepOutcome)
    -> Result<()>;
```

#### 2.4 `htui-store/src/pg/write.rs`

Add `StepFence` to the `use htui_core::store::{…}` list (`:44-54`). `:8-9`: replace "exactly as the
pending-buffer upload does (`crate::cache::pending`), and" with "and".

**`append_events`** (`:928-973`), whole replacement:

```rust
    /// Appends session events in one statement under `fence`, skipping the `(run_step_id, seq)`
    /// pairs already stored (ANA-9 §4.3, plan D3; MOD-40 plan D1).
    ///
    /// The batch is carried as **one** `jsonb` parameter and expanded by `jsonb_to_recordset`,
    /// rather than as nine parallel arrays: `SessionEvent`'s serde form already *is* the row -
    /// field names are the column names verbatim - and nullable `text[]` / `jsonb[]` parameters
    /// are avoided entirely.
    ///
    /// The fence is decided **inside** the statement (MOD-40 blueprint B1, B2). `lease` reads the
    /// named steps' runs and share-locks them, so a `take_lease` or `adopt_runs` either waits for
    /// this write or is seen by it; `fenced` is the lowest step whose run's `lease_owner` is not
    /// `$2`. The insert runs only when nothing is fenced, **or** when a named step does not exist
    /// at all, so that such a row still reaches the foreign key and refuses the batch (`23503`)
    /// ahead of the fence. The statement answers both facts, so no second read is needed and none
    /// can race.
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
                   FOR SHARE OF r
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
```

P-1, on a run leased to `a1` (step `aa`) and a chat run (`NULL`, step `cc`): (A) `a1`, 2 new rows
→ `(2, NULL)`; (B) `a1`, 2 stored + 1 new → `(1, NULL)`; (C) stale `b1` → `(0, aa)`; (D) `NULL`
on `aa` → `(0, aa)`; (E) `NULL` on `cc` → `(1, NULL)`; (F) `a1` on `cc` → `(0, cc)`; (G) missing
step → `23503`; (H) `[aa, missing]` under `a1` → `23503`, nothing lands; (I) the same under `b1` →
`23503` (B6); (J) `a1`, all stored → `(0, NULL)`; (K) bad `kind` under `a1` → `23514`.
`$2` binds `Option<Uuid>`: precedent `new.produced_by_step_id.map(StepId::as_uuid)`
(`write.rs:221`). Result columns are `bigint` and `uuid` (P-2), hence `"inserted!"` (`i64`) and
`"fenced_step?"` (`Option<Uuid>`).

**`set_step_usage`** (`:975-1011`). Doc: keep the first two paragraphs; replace `# Errors` with
"[`StoreError::NotFound`] when the step does not exist and [`StoreError::Fenced`] when it does and
its run does not carry `fence`'s lease, told apart by `step_exists`' follow-up read on a miss
(`interrupt_step`'s shape). `FOR SHARE` on the run for `append_events`' reason (MOD-40 blueprint
B2)." Body:

```rust
    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        let updated = sqlx::query!(
            "UPDATE run_step \
                SET usage = $2, prompt_digest = COALESCE($3, prompt_digest) \
              WHERE id = $1 \
                AND EXISTS (SELECT 1 FROM run r \
                             WHERE r.id = run_step.run_id \
                               AND r.lease_owner IS NOT DISTINCT FROM $4 \
                               FOR SHARE)",
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
        let mut conn = self.pool.acquire().await.map_err(map_sqlx)?;
        step_exists(&mut conn, step).await?;
        Err(StoreError::Fenced { step })
    }
```

**`finish_step`** (`:4060-4102`). Doc: replace `# Errors` as for `set_step_usage`. Body: signature
`finish_step(&self, fence: StepFence, step: StepId, outcome: StepOutcome)`; the `WHERE` becomes

```sql
              WHERE id = $1
                AND EXISTS (SELECT 1 FROM run r
                             WHERE r.id = run_step.run_id
                               AND r.lease_owner IS NOT DISTINCT FROM $8
                               FOR SHARE)
```

with `fence.owner()` as the eighth argument after `outcome.finished_at`, and the tail
`if updated == 0 { NotFound }` replaced by the `set_step_usage` tail above (`== 1` → `Ok`, else
`step_exists` → `NotFound`, else `Fenced { step }`).

**Today's missing-step answers stay**: append → `23503` → `map_sqlx` → `Constraint`
(`htui-store/src/error.rs:42-45`), as the suite asserts (`conformance.rs:1356-1363`, `:1443-1447`);
updates → `NotFound { entity: "run_step" }` (`conformance.rs:1408-1418`, `:7536-7548`).

#### 2.5 `htui-core/src/store/mem.rs`

`:14`: `use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};`. Add `StepFence` to the
`crate::store::traits::{…}` import (`:47-49`). In `impl State`, before `append_events` (`:1430`):

```rust
    /// MOD-40 plan D1: whether `row`'s run carries `fence`'s lease. `lease_owners` holds a run only
    /// while its lease names an owner ([`WriteStore::release_lease`] removes it), which is
    /// Postgres's `lease_owner IS NOT DISTINCT FROM $fence`. A function of the map rather than of
    /// `self`, so a caller can hold `steps` mutably while it asks.
    fn fence_holds(
        lease_owners: &HashMap<RunId, Uuid>,
        row: &RunStep,
        fence: StepFence,
    ) -> Result<()> {
        if lease_owners.get(&row.run_id).copied() == fence.owner() {
            Ok(())
        } else {
            Err(StoreError::Fenced { step: row.id })
        }
    }
```

`append_events` (`:1435`): signature `fn append_events(&mut self, fence: StepFence, events:
&[SessionEvent]) -> Result<usize>`; doc gains "then the fence (MOD-40 plan D1), lowest step first,
both before the first insert". Between the existence loop (`:1436-1443`, unchanged, first) and the
insert loop, insert:

```rust
        // Then the fence, before any row is written; the lowest step first, which is the one
        // Postgres's `fenced` CTE names (MOD-40 blueprint B6).
        let named: BTreeSet<StepId> = events.iter().map(|event| event.run_step_id).collect();
        for step in named {
            if let Some(row) = self.steps.get(&step) {
                Self::fence_holds(&self.lease_owners, row, fence)?;
            }
        }
```

`set_step_usage` (`:1462`): add `fence: StepFence` first; after the existing `get_mut … ?`
(`:1469-1475`) insert `Self::fence_holds(&self.lease_owners, row, fence)?;` before the first
assignment. `finish_step` (`:4217`): the same, after its `get_mut … ?` (`:4223-4229`). The borrow
is disjoint (`steps` mutably, `lease_owners` shared); validate-before-write holds because the check
precedes every assignment.

Wrappers: `:5582-5584` → `async fn append_events(&self, fence: StepFence, events:
&[SessionEvent])` calling `state.append_events(fence, events)`; `:5586-5594` and `:6002-6005`
take `fence: StepFence` first and pass it first.

Unit tests: `:6219` gains `StepFence`; `:6301`, `:6309`, `:6344`, `:8556`, `:8570`, `:8610` pass
`StepFence::Unleased` first (fixture steps of `RUN_1`/`RUN_2`, never claimed).

#### 2.6 `htui-store/src/writer.rs`

Add `StepFence` to `use htui_core::store::{…}` (`:52-55`). `:378-393` and `:933-938` take
`fence: StepFence` first and forward it first to both arms.

Module doc `:8-27`, current (abridged): "…`BufferedWriter` is a `CacheStore` handle and a path…
There are **three** arms, but since MOD-25 only two of them are ever constructed. … `upload_pending`
still runs on every refresh pass so buffers from earlier builds land. A later CLEAN item deletes
all of it. …" Replacement for `:8-27`:

```rust
//! [`Writer`] is that something. Both arms are cheap handles — `PgStore` is a pool handle and
//! `MemStore` is an `Arc` — so a `Writer` is a clone of a handle, never a copy of a store.
//!
//! **Online only (MOD-25).** Between MOD-2 milestone 4 and MOD-25 a third arm buffered an offline
//! chat's rows under `<cache_dir>/pending/` for a later upload (plan D34). MOD-25 removed that
//! path: [`Backend::writer`](crate::Backend::writer) answers `None` off the server, where a chat is
//! refused with [`DATABASE_UNREACHABLE`], and [`Backend::writable`](crate::Backend::writable)
//! still answers `None` off the server, so nothing writes to Postgres unless the backend is
//! `Online`. A step write the store refuses is re-offered by the recorder itself, at the same
//! `seq` (`htui_agent::record`, MOD-40 plan D3); no buffer outlives the process.
```

`:82-113` (the orphaned `BufferedWriter` doc: "/// The offline [`WriteStore`] (MOD-2 plan D34,
D35): …" through "/// D35's refusal for the three item writes: offline item editing is MOD-13's
question.\n///"): **delete**. `REGISTRY_ON_SERVER_ONLY`'s own doc (`:114-119`) stays.

#### 2.7 Test doubles

- `htui-agent/src/conformance.rs` (`UsageSpy`): `:52-53` import `StepFence`; `:726-728`,
  `:729-745`, `:1093-1095` take `fence: StepFence` first and forward it.
- `htui-agent/tests/recorder.rs` (`SpyStore`): `:52-55` import `StepFence`; `:406-414`,
  `:415-432`, `:787-789` the same. (T2 extends `append_events`, §3.4.)

#### 2.8 D2 plumbing (moved here by B3)

**`htui-agent/src/record.rs`**
- `:106`: `use htui_core::store::{StepFence, StoreError, WriteStore};`
- Field, after `run_cap` (`:418`):
  ```rust
      /// MOD-40 plan D2: the lease every row, usage and digest write of this session is made
      /// under. [`StepFence::Unleased`] unless [`Recorder::with_fence`] says otherwise.
      fence: StepFence,
  ```
- `new` (`:458-491`): `fence: StepFence::Unleased,` after `run_cap: None,`. `continuing` inherits
  it through `..Self::new(…)` (`:520-527`), which is what a promoted step's chat needs.
- Debug (`:427-449`): `.field("fence", &self.fence)` after `.field("run_cap", &self.run_cap)`.
- Builder, after `with_run_cap` (`:555-559`):
  ```rust
      /// Writes every row, the usage and the digest under `fence` (MOD-40 plan D2).
      ///
      /// A builder for [`Recorder::with_quota_latch`]'s reason: most recorders are a chat's, whose
      /// run holds no lease, and [`StepFence::Unleased`] is the default. The engine passes the lease
      /// its walk holds. A recorder that forgets it on a leased run is refused with
      /// [`StoreError::Fenced`] at its first write, loudly; one whose lease another process took
      /// writes nothing more.
      #[must_use]
      pub fn with_fence(mut self, fence: StepFence) -> Self {
          self.fence = fence;
          self
      }
  ```
- `:1042`: `self.store.append_events(self.fence, &rows)` (T2 rewrites the whole `flush`, §3.2).
- `:1114`: `self.store.set_step_usage(self.fence, self.step, usage, digest).await?;`
- Unit tests: `:1945` and `:2129` pass `StepFence::Unleased` first (chat steps); add
  `StepFence` to the module's `use htui_core::store::{…}`.

**`htui-orch/src/engine.rs`**
- `:44`: `use htui_core::store::{StepFence, StoreError, WriteStore};`
- `open_recorder` (`:5265-5292`): the `Recorder::new(…)` expression (`:5274-5280`) gains
  ```rust
          )
          // MOD-40 plan D2: the walk's lease rides every row, usage and digest write, so a walk
          // that sleeps through its lease writes nothing once another process has adopted it.
          .with_fence(StepFence::Lease(self.parts.owner));
  ```
- `finish_step`: insert `StepFence::Lease(self.parts.owner),` as the first argument at `:1404`
  (accept, under `take_lease` `:1369`), `:2283` (`finish_recovered`, after `adopt_runs`), `:3190`
  (`walk_step` settle), `:3791` (candidate settle), `:4232` (`settle_judge`). Every one runs under
  a lease this engine's `owner` took.

#### 2.9 T1 call sites, complete

`grep -rn '\.append_events(\|\.set_step_usage(\|\.finish_step(' crates --include=*.rs`:

| Site | Fence | Why |
|---|---|---|
| `htui-orch/src/engine.rs:1403, 2282, 3189, 3790, 4231` | `Lease(self.parts.owner)` | §2.8 |
| `htui-agent/src/record.rs:1042, 1114` | `self.fence` | §2.8 |
| `htui-agent/src/record.rs:1945, 2129` (tests) | `Unleased` | chat step (`open_step`, `:1857-1871`) |
| `htui-agent/src/conformance.rs:727, 738, 1094` | forwarded | `UsageSpy` |
| `htui-agent/tests/recorder.rs:413, 422, 788` | forwarded | `SpyStore` |
| `htui-store/src/writer.rs:380-381, 392-393, 935-936` | forwarded | `Writer` |
| `htui-core/src/store/mem.rs:5583, 5593, 6004` | forwarded | wrappers |
| `htui-core/src/store/mem.rs:6301, 6309, 6344, 8556, 8570, 8610` (tests) | `Unleased` | `STEP_IMPL`, `STEP_R2_PRD`, unclaimed |
| `htui-core/src/store/conformance.rs:1319, 1332, 1359, 1377` | `Unleased` | `STEP_IMPL` of `RUN_1` (no lease, `fixtures.rs:1418`) |
| `htui-core/src/store/conformance.rs:1396, 1404, 1408` | `Unleased` | same |
| `htui-core/src/store/conformance.rs:1443, 1456` | `Unleased` | chat run |
| `htui-core/src/store/conformance.rs:7204` | `Unleased` | `interrupt_step…` creates a run it never claims (`:7148-7152`) |
| `htui-core/src/store/conformance.rs:7472, 7509, 7537` | `Unleased` | `STEP_R2_PRD`, `RUN_2` queued |
| `htui-store/tests/pg_criteria.rs:1015, 1032, 1050, 1097` | `Unleased` | `STEP_IMPL` |
| `htui/tests/chat.rs:756` | `Unleased` | fixture `STEP_PRD` of `RUN_1` |
| `htui/tests/runs_pg.rs:647` | `Unleased` | the run is parked (`:594`), and a park releases the lease (D139) before `Stack::command` returns (`:292-299` drives the reply) |

Imports: `conformance.rs:37-41` (`crate::store::traits::{…}`), `pg_criteria.rs:29-32`,
`htui/tests/chat.rs:41`, `htui/tests/runs_pg.rs:54` gain `StepFence`. No production caller in
`htui/src` calls the three methods; chats (`agent_worker.rs:3102`, `:3107`) keep the default.

#### 2.10 T1 tests — store conformance (both stores)

Append the six names to `CASES` after `"skill_binding_stores_expanded_globs_and_languages_as_typed"`
(`conformance.rs:127`) in this order, add six `run_case` arms before `other => panic!` (`:290`),
and put the bodies after `release_lease_frees_the_run_for_its_own_sweep` (before `fn fixture_box`,
`:5466`). Shared helpers (private, beside them):

```rust
/// MOD-40 D1: a fresh run claimed by `a` until `at + 5 min`, with one step. The fence cases start
/// here; the run is `HTUI_ANA_2`'s, as the lease cases' are.
async fn leased_step<S: WriteStore>(case: &str, store: &S, a: Uuid, at: DateTime<Utc>)
-> (RunId, StepId) { /* create_run(new_run(PROJECT_HTUI, HTUI_ANA_2, vec![])); claim_run(run,
   BOX, a, at, at + 5 min) == Claim::Admitted; create_step(new_run_step(run, 0, 1, 0)) */ }

/// B takes the run's lapsed lease, as a sweep's `adopt_runs` does to a suspended holder.
async fn taken_by<S: WriteStore>(case: &str, store: &S, run: RunId, b: Uuid, at: DateTime<Utc>)
{ /* assert take_lease(run, BOX, b, at + 6 min, at + 20 min) */ }

/// The step's log, empty when nothing is cached (`step_events` answers `None` then).
async fn log_of<S: ReadStore>(case: &str, store: &S, step: StepId) -> Vec<SessionEvent>
{ /* step_events(step).expect(case).unwrap_or_default() */ }

/// The three fenced writes under `fence` each answer `Fenced { step }`: an append at `seq`, a
/// usage write, a settle at `at`.
async fn assert_fenced<S: WriteStore>(case: &str, store: &S, fence: StepFence, step: StepId,
    seq: i32, at: DateTime<Utc>) { /* three matches!(…, Err(StoreError::Fenced { step: s }) if s
    == step) with "{case}: …" messages */ }
```

| Case | Setup | Asserts |
|---|---|---|
| `a_stale_owner_writes_nothing_to_the_step` | `a`, `b` = `Uuid::now_v7()`; `at = seam_clock()`; `leased_step` as A; A appends seq 0..2 (`Lease(a)`) → `2`; A's `set_step_usage(Lease(a), json!({"input_tokens": 1}), Some("d0"))`; `taken_by` B. Snapshot `row = step_row(..)`, `log = log_of(..)`. | `assert_fenced(Lease(a), step, 2, at + 7 min)`; A's **replay** of seq 0..2 under `Lease(a)` → `Fenced` (B7); `step_row == row` (settle and usage untouched); `log_of == log` (2 rows). |
| `the_new_owner_writes_the_step` | as above through `taken_by`. | B appends seq 2..4 (`Lease(b)`) → `2`; `set_step_usage(Lease(b), json!({"input_tokens": 5}), None)` Ok; `finish_step(Lease(b), StepOutcome { exit_code: Some(0), finished_at: at + 7 min, .. })` Ok; row: `exit_code == Some(0)`, `finished_at == Some(at + 7 min)`, `usage == Some(json!({"input_tokens": 5}))`; log seqs `0..4`. |
| `an_unleased_fence_writes_a_chat_step` | `ChatRunSpec::mint(PROJECT_HTUI, BOX, USER, Some(AGENT_CLAUDE), Some("sonnet"))`; `start_chat_run`. | `Unleased`: append 0..3 → `3`, usage Ok, `finish_step` Ok. Then `assert_fenced(Lease(Uuid::now_v7()), chat.step_id, 3, ..)`; row and log unchanged by the refusals (`step_row(case, store, chat.run_id, chat.step_id)`). |
| `an_unleased_fence_is_refused_on_a_leased_run` | `leased_step` as A. | `assert_fenced(Unleased, step, 0, ..)`; `log_of` empty, `finished_at == None`. Then `release_lease(run, a, at + 1 min)` → `true`; now `Unleased` append seq 0..1 → `1` (a released park is a chat's to continue) and `assert_fenced(Lease(a), step, 1, ..)`. |
| `a_replayed_batch_under_the_right_fence_is_ok_zero` | `leased_step` as A. | `Lease(a)`: seq 0..3 → `3`; the same → `0`; seq 0..4 → `1`; log seqs `0..4`. `Unleased` replay of 0..3 → `Fenced` (B7). |
| `a_missing_step_keeps_its_old_error_not_fenced` | `orphan = StepId::new()`; `leased_step` as A; `taken_by` B. | For each of `Unleased`, `Lease(Uuid::now_v7())`: append on `orphan` → `Constraint`; `set_step_usage`/`finish_step` on `orphan` → `NotFound { entity: "run_step", .. }`. Mixed batch `[chat_event(step, 0), chat_event(orphan, 0)]` under stale `Lease(a)` → `Constraint` (B6, P-1 I), `log_of(step)` empty. |

Existing cases keep every assertion, fence added (§2.9).

**Postgres-only** (`htui-store/tests/pg_criteria.rs`, `#[tokio::test(flavor = "multi_thread")]`),
`a_lease_take_committed_mid_write_fences_it` (pins B2): `demo_db()`; A claims `ids::RUN_2`
(`claim_run(RUN_2, BOX, a, now, now + 5 min)` → `Admitted`, P-5's fixture on Postgres); on a
separate connection `let mut tx = db.pool.begin()`, `sqlx::query("UPDATE run SET lease_owner = $1
WHERE id = $2").bind(b).bind(RUN_2.as_uuid()).execute(&mut *tx)` (runtime query: no `.sqlx`
file); `tokio::spawn` a clone of `db.store` appending one row to `STEP_R2_PRD` under `Lease(a)`;
`tokio::time::sleep(300 ms)`; assert `!handle.is_finished()` ("the write waits on the take's row
lock"); `tx.commit()`; the join answers `Err(Fenced { step: STEP_R2_PRD })`; `step_events` is
`None`.

**Pins**: `htui-core/tests/mem_store.rs:37` 83 → 89, message gains ", and MOD-40 milestone 1's six
for the step fence (plan D1)"; `htui-store/tests/pg_conformance.rs:19` `EXPECTED_CASES` 83 → 89.

### 3. T2 — replay and fresh rows (D3), the tests

#### 3.1 `record.rs` fields and docs

Replace `unflushed`'s doc (`:387-397`) and add `unoffered` after it (`:398`):

```rust
    /// Rows offered to the store once, by a flush the store answered with an error (MOD-40 plan
    /// D3). The error may have come after the commit (a reply the connection lost), so some or
    /// all of them may be stored already. The next flush re-offers them **alone**, first, at the
    /// `seq` they were given, and a short count there is the replay finding its own rows.
    ///
    /// Not necessarily ascending in `seq`, which is safe: `append_events` inserts from an explicit
    /// `seq` with `ON CONFLICT (run_step_id, seq) DO NOTHING`, `MemStore` does the same scan, and
    /// every reader orders by `seq`.
    unflushed: Vec<SessionEvent>,
    /// Rows numbered and owed but **never offered** (MOD-40 blueprint B4): a held `edit_proposal`
    /// whose call closed ([`Recorder::release_held`]), and a batch a flush numbered and could not
    /// offer because the replay ahead of it was refused. They go out in the next flush's fresh
    /// call, where every row must land.
    unoffered: Vec<SessionEvent>,
```

`new`: `unoffered: Vec::new(),` after `unflushed`. Debug: `.field("unoffered",
&self.unoffered.len())` after `unflushed`. `release_held` (`:1102`): `self.unoffered.push(event);`
and its doc sentence "The row goes to `unflushed`" (`:1068`) → "The row goes to `unoffered`".

`RecordError::Store` doc (`:240-245`) becomes:

```rust
    /// The store refused a write, or a fresh batch collided (MOD-40 plan D3).
    ///
    /// A **refusal** — any store error — leaves the rows that flush had numbered owed inside the
    /// recorder, re-offered at those same numbers by the next flush (a retry, or
    /// [`Recorder::finish`]); dropping the recorder is what loses them. A step whose run another
    /// process adopted answers [`StoreError::Fenced`] here, and so does every later write.
    ///
    /// A **collision** is [`StoreError::Constraint`] naming `seq`s "already held: a second writer
    /// on this step": rows the recorder had never offered came back short, so another writer holds
    /// those numbers. They are **not** owed again; re-offering them would be a replay and would
    /// hide the second writer.
```

`RecorderSummary::rows` (`:274`): "Rows the store accepted. Lower than `seq` when a replay found
rows already stored, or when a fresh batch collided (reported as an error)."

`record_unreadable`'s doc (`:1437`): "keeps its numbered rows in `unflushed`" → "keeps its
numbered rows owed". Its warn text (`:1475-1476`): F-12.

#### 3.2 `flush` (`record.rs:996-1052`), whole replacement

```rust
    /// Writes what is buffered and closes the open run.
    ///
    /// The final scrub happens here rather than at capture, because a secret split across two
    /// chunks only exists once the run is assembled; masking is idempotent, so scrubbing the
    /// already-scrubbed pieces again costs a pass and changes nothing.
    ///
    /// **Two calls, replay first (MOD-40 plan D3).** Rows a refused call already offered
    /// (`unflushed`) go out alone: some may be stored, so a short count is fine. Then the fresh rows
    /// — `unoffered` and the buffer, numbered here — go out together, and every one must land: a
    /// short count means another writer holds those `seq`s, which is reported as a
    /// [`StoreError::Constraint`] and **not** re-queued. A store error on either call keeps its rows
    /// owed at their numbers, so with one writer a failed append costs a retry and never a hole.
    ///
    /// `next_seq` advances at **numbering** time, because since plan D77 a held `edit_proposal`
    /// reserves one at its announcement: a refused batch keeps the numbers it was given, and
    /// advancing only on success would let a reservation collide with them. A collided `seq` stays
    /// spent, so the recorder never offers it again. `rows` grows by what each call reports landed.
    ///
    /// **What it does not do is clear the held rows.** That was T46's defect: `(tool_call_id, path)`
    /// identity is a domain fact and a flush is a persistence detail.
    async fn flush(&mut self) -> Result<(), RecordError> {
        self.buffer_kind = None;
        self.open_message_id = None;
        if self.buffer.is_empty() && self.unflushed.is_empty() && self.unoffered.is_empty() {
            return Ok(());
        }

        let pending = core::mem::take(&mut self.buffer);
        let mut fresh = core::mem::take(&mut self.unoffered);
        fresh.reserve(pending.len());
        for mut row in pending {
            let outcome = self.scrubber.scrub(&mut row.payload);
            let row = match outcome {
                Ok(()) => row,
                Err(unmasked) => {
                    self.note_residue(&unmasked);
                    residue_row(&unmasked, row.at)
                }
            };
            let seq = self.next_seq;
            self.next_seq += 1;
            let turn = self.turn;
            fresh.push(self.event_row(row, seq, turn));
        }

        if !self.unflushed.is_empty() {
            let replay = core::mem::take(&mut self.unflushed);
            match self.store.append_events(self.fence, &replay).await {
                Ok(written) => self.rows += written,
                Err(error) => {
                    self.unflushed = replay;
                    self.unoffered = fresh;
                    return Err(RecordError::Store(error));
                }
            }
        }

        if fresh.is_empty() {
            return Ok(());
        }
        match self.store.append_events(self.fence, &fresh).await {
            Ok(written) => {
                self.rows += written;
                if written == fresh.len() {
                    Ok(())
                } else {
                    Err(RecordError::Store(StoreError::Constraint(seq_collision(
                        self.step, &fresh, written,
                    ))))
                }
            }
            Err(error) => {
                self.unflushed = fresh;
                Err(RecordError::Store(error))
            }
        }
    }
```

Beside `residue_row` (`:1573`):

```rust
/// The refusal a fresh batch that landed short answers with (MOD-40 plan D3): some `seq` it
/// numbered is already held on the step, which cannot happen with one writer.
fn seq_collision(step: StepId, fresh: &[SessionEvent], written: usize) -> String {
    let low = fresh.iter().map(|row| row.seq).min().unwrap_or_default();
    let high = fresh.iter().map(|row| row.seq).max().unwrap_or_default();
    format!(
        "session_event seq {low}..={high} of run_step `{step}`: {written} of {} landed, the rest \
         already held: a second writer on this step",
        fresh.len()
    )
}
```

**Accounting.** `rows` is the sum of what the store reported landed, per call; a replay's
already-stored rows and a collision's taken rows are not counted. `next_seq` is untouched by the
calls: every row was numbered once, before either call; a refused batch keeps its numbers
(`unflushed`/`unoffered`), a collided batch's numbers stay spent. **Held rows** (B4) are never
replay: they enter `unoffered` and ride the fresh call, so a short count on one of them is a
collision, reported as such.

#### 3.3 Tests — `htui-orch` conformance (B5, B9)

`htui-orch/src/fake.rs`:
- `stalls` (`:1279`): `Mutex<BTreeMap<ScriptKey, (bool, Arc<Notify>, Option<Arc<Notify>>)>>`; doc
  adds "and, for a suspend, the signal that wakes it".
- `stall_after_done` (`:1462-1478`) inserts `(write_output, Arc::clone(&notify), None)`;
  `take_stall` (`:1480-1494`) returns the triple.
- New, after `stall_after_done`:
  ```rust
      /// MOD-40 T2: [`stall_after_done`](Self::stall_after_done) with no document, as a
      /// **suspend** rather than a death — the session signals the first [`Notify`], then awaits
      /// the second and returns `Ok(None)`, and the walk settles on. A laptop lid, not a kill.
      ///
      /// # Panics
      /// When a lock is poisoned, which no case does.
      #[must_use]
      pub fn suspend_after_done(&self, phase: &str, attempt: i32, slot: Option<(i32, u32)>)
      -> (Arc<Notify>, Arc<Notify>) { /* insert (false, stalled, Some(wake)) */ }
  ```
- `after_done` (`:1616-1622`):
  ```rust
          if let Some((write_output, stalled, wake)) = self.take_stall(key) {
              if write_output {
                  self.write_output(item, step, phase, key).await?;
              }
              stalled.notify_one();
              let Some(wake) = wake else {
                  return std::future::pending().await;
              };
              wake.notified().await;
              return Ok(None);
          }
  ```

`htui-orch/src/conformance.rs`:
- `:29` gains `StepFence`. Use full paths for `htui_core::store::StoreError`,
  `htui_agent::record::{Recorder, RecordError}` and `htui_core::scrub::MinimalScrubber`, as the
  file already does (`:179`, `:6473`, `:6510`); importing `StoreError` would trip
  `unused_qualifications` on `:179`, `:266`.
- `Orchestrate` (after `stall_after_done`, `:157-165`) gains `fn suspend_after_done(&self, phase:
  &str, attempt: i32, slot: Option<(i32, u32)>) -> (Arc<Notify>, Arc<Notify>);` with the fake's
  doc, `#[must_use]`; the impl (`:246-254`) forwards.
- `CASES` (`:504`): append `// MOD-40 plan D1, D2: a walk that wakes after another process adopted
  its run writes nothing to the step.` and `"a_suspended_walk_cannot_write_after_adoption",`.
  `case` (`:713-716`) gains the arm. Doc `:288-292`: "Seventy-three"; "18 + 5 + 13 + 6 + 10 + 18 + 2
  + 1"; add "**One for MOD-40 milestone 1** (plan D1, D2): a suspended walk, woken after another
  process adopted its run, writes nothing to the step." `:520`: "seventy-three-arm".
- Pins: `cases_are_unique_and_counted` (`:5756-5780`) 72 → 73, message gains ", and MOD-40
  milestone 1's one (a suspended walk fenced after adoption)"; `htui-orch/tests/fake_conformance.rs:16`
  72 → 73.

Body (place after `a_live_lease_blocks_an_answer_from_another_process`, before the recovery
section, `:4048`):

```rust
/// MOD-40 plan D1, D2 (ANA-16 C1): a walk suspended between its session and its settle, whose run
/// another process adopted meanwhile, wakes and writes nothing to the step. Its `finish_step` is
/// fenced by the owner it walked under, and so is a recorder built the way the engine builds one.
/// The adopter's row and log stand.
async fn a_suspended_walk_cannot_write_after_adoption<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    let (stalled, wake) = orch.suspend_after_done("prd", 1, None);
    let mut walk = Box::pin(orch.dispatch(Command::StartRun {
        item: ids::HTUI_FEAT_3,
        mode: RunMode::Manual,
        repo_scope: None,
    }));
    if let Either::Left(_) =
        futures::future::select(walk.as_mut(), Box::pin(stalled.notified())).await
    {
        panic!("the walk finished instead of suspending");
    }
    let run = orch.store().runs(ids::HTUI_FEAT_3).await.expect("MemStore never fails a read")
        .into_iter().find(|run| run.status == RunStatus::Running)
        .expect("the suspended walk left the run `running`").id;
    let prd = step_at(&orch, run, 0, 1).await;

    let other = orch.restarted();
    assert_eq!(other.sweep().await.expect("the sweep adopts"), [Adopted { run, next: Next::Walk }]);
    let adopted = step_at(&other, run, 0, 1).await;
    assert_eq!(
        (adopted.status, adopted.gate_note.as_deref()),
        (StepStatus::Failed, Some("interrupted")),
        "the adopter settled `prd` as interrupted (plan D89)"
    );
    let log = orch.store().step_events(prd.id).await.expect("MemStore never fails a read");

    wake.notify_one();
    let woke = walk.await;
    assert!(
        matches!(&woke, Err(EngineError::Store(htui_core::store::StoreError::Fenced { step }))
            if *step == prd.id),
        "{woke:?}"
    );
    assert_eq!(step_at(&other, run, 0, 1).await, adopted, "the woken settle wrote nothing");
    assert_eq!(orch.store().step_events(prd.id).await.expect("MemStore never fails a read"), log);

    let scrubber = htui_core::scrub::MinimalScrubber::new([]);
    let mut recorder = htui_agent::record::Recorder::new(orch.store(), &scrubber, prd.id, false, None)
        .with_fence(StepFence::Lease(orch.owner()));
    let refused = recorder
        .record_prompt("woken", serde_json::json!([]), orch.clock().now())
        .await;
    assert!(
        matches!(&refused, Err(htui_agent::record::RecordError::Store(
            htui_core::store::StoreError::Fenced { step })) if *step == prd.id),
        "{refused:?}"
    );
    assert_eq!(orch.store().step_events(prd.id).await.expect("MemStore never fails a read"), log);
    assert!(
        other.store().refresh_lease(run, other.owner(), other.clock().now() + TimeDelta::minutes(1))
            .await.expect("MemStore refreshes"),
        "the adopter still holds the lease"
    );
}
```

Why it holds: A's woken tail is verify → `capture` → `record_commits` (unfenced, F-7) →
`finish_step(Lease(A))` (`engine.rs:3159-3189`), all ready futures on the fake, polled before the
heartbeat (`select` polls the walk first, `engine.rs:1720`; the heartbeat sleeps in real time).
`walk_leased`'s `Err` path releases A's lease best-effort (`:1683-1688`), which B holds, so it writes
nothing. The engine's own recorder fence needs no case: without `.with_fence` every claimed walk is
`Fenced` at `record_prompt`, and 72 cases fail.

#### 3.4 Tests — recorder (`htui-agent/tests/recorder.rs`, B8)

`SpyStore` (`:195-222`) gains, with `demo()` initialisers:

```rust
    /// Every `append_events` call that reached the inner store, as `(fence, seqs)`, in order.
    appends: Mutex<Vec<(StepFence, Vec<i32>)>>,
    /// Appends still to be let through and then answered `Unreachable`: a commit whose reply the
    /// connection lost (MOD-40 plan D3).
    lose_replies: Mutex<usize>,
```

helpers `fn appends(&self) -> Vec<(StepFence, Vec<i32>)>` and `fn lose_next_append_replies(&self,
count: usize)` (the `refuse_next_appends` shape, `:266-285`), and `append_events` (`:406-414`):

```rust
    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> StoreResult<usize> {
        // The guards are dropped before and taken after the await: none is held across one.
        if self.refuses_this_append() {
            return Err(StoreError::Unreachable("the spy store refused this append".to_owned()));
        }
        let written = self.inner.append_events(fence, events).await?;
        self.appends.lock().expect("the spy log is never poisoned")
            .push((fence, events.iter().map(|row| row.seq).collect()));
        if self.loses_this_reply() {
            return Err(StoreError::Unreachable("the spy store lost this append's reply".to_owned()));
        }
        Ok(written)
    }
```

Imports: `chrono::{DateTime, TimeDelta, Utc}`; `StepFence` (T1).

| Test | Script | Asserts |
|---|---|---|
| `a_fresh_batch_that_lands_short_is_an_error_and_not_requeued` | `open_chat`; a second writer's row at seq 0 via `store.inner.append_events(Unleased, &[…seq 0, payload {"text": "the other writer"}])`; `Recorder::new`; `record_prompt("summarise the backlog", json!([]), at())`. | `Err(RecordError::Store(StoreError::Constraint(m)))` with `m` containing `"already held"` and `"a second writer on this step"`. Then `record(chunk("mine", "m1"))`, `finish()` → `Ok`. `store.appends() == [(Unleased, vec![0]), (Unleased, vec![1])]` ("seq 0 is never offered again"); log seqs `[0, 1]`, `log[0].payload == json!({"text": "the other writer"})`; `summary.rows == 1`, `summary.seq == 2`. |
| `a_replayed_batch_may_land_short` | `open_chat`; `Recorder::new`; `store.lose_next_append_replies(1)`; `record_prompt(…)`. | `Err(RecordError::Store(StoreError::Unreachable(_)))`; the log already holds the prompt at seq 0. Then `record(chunk("an answer", "m1"))`, `finish()` → `Ok` (the replay's short count is not a collision). `store.appends() == [(Unleased, [0]), (Unleased, [0]), (Unleased, [1])]` ("the lost batch, its replay alone, then the fresh row alone"); log kinds `[Prompt, AssistantText]`, seqs `[0, 1]`; `summary.rows == 1`, `summary.seq == 2`. |
| `the_fence_rides_every_write` | `owner`, `stranger` = `Uuid::now_v7()`; `store.inner.claim_run(ids::RUN_2, ids::BOX, owner, at(), at() + 5 min)` → `Admitted` (P-5). | (1) Default fence: `Recorder::new(&store, …, ids::STEP_R2_PRD, …)`, `record(chunk("x", "m1"))`, `record(done())` → `Err(RecordError::Store(StoreError::Fenced { step: STEP_R2_PRD }))`. (2) `.with_fence(StepFence::Lease(owner))`: `record(chunk)`, `record(usage(Some(9), None))`, `record(done())`, `finish()` → `Ok`; every `appends()` entry of this recorder is `(Lease(owner), _)`; the run's `STEP_R2_PRD` row has `usage == Some(summary.usage)` (the usage write rode the fence: under `Unleased` it would be `Fenced`). (3) `store.inner.take_lease(RUN_2, BOX, stranger, at() + 6 min, at() + 20 min)` → `true`; a `Recorder::continuing(…, &tail).with_fence(Lease(owner))`, `record(chunk)`, `record(done())` → `Fenced`; the log is unchanged. |

Existing recorder tests keep their assertions: a refused call wrote nothing, so the replay lands
whole and `rows == seq` holds (`:3208-3256`, `:2236-2276`, `:2350-2420`).

### 4. Doc fixes carried by T1/T2

T1: `traits.rs:256-264`, `:285-296`, `:298-306`, `:1078-1081` (§2.3); `pg/write.rs:8-9`, `:928-945`,
the two `# Errors` (§2.4); `writer.rs:8-27`, `:82-113` (§2.6). T2: `record.rs:240-245`, `:274`,
`:387-397`, `:996-1017`, `:1068`, `:1437`, `:1475-1476` (§3.1, §3.2). Plan R-1: one sentence for
F-11. The MOD-41 note: F-7.

### 5. File-by-file checklist

**T1**
- [ ] `htui-core/src/store/traits.rs` — `StepFence` + `owner()`; three signatures; docs (§2.1, §2.3)
- [ ] `htui-core/src/store/mod.rs` — re-export `StepFence`
- [ ] `htui-core/src/store/error.rs` — `Fenced { step }`, import `StepId`
- [ ] `htui-core/src/store/mem.rs` — `fence_holds`, three inner fns, three wrappers, `BTreeSet`, six test sites
- [ ] `htui-core/src/store/conformance.rs` — 13 call sites (§2.9); six cases + helpers + arms + `CASES`
- [ ] `htui-core/tests/mem_store.rs:37` — 89
- [ ] `htui-store/src/pg/write.rs` — three statements, docs, import
- [ ] `htui-store/src/writer.rs` — three forwards, module doc, delete `:82-113`
- [ ] `htui-store/.sqlx/` — `-query-c42d3f5c…`, `-query-ad388b2a…`, `-query-de3b2558…`, +3; 281 files
- [ ] `htui-store/tests/pg_conformance.rs:19` — 89
- [ ] `htui-store/tests/pg_criteria.rs` — four sites; `a_lease_take_committed_mid_write_fences_it`
- [ ] `htui-agent/src/conformance.rs` — `UsageSpy` forwards
- [ ] `htui-agent/tests/recorder.rs` — `SpyStore` forwards
- [ ] `htui-agent/src/record.rs` — `fence` field, `new`, Debug, `with_fence`, `:1042`, `:1114`, two test sites
- [ ] `htui-orch/src/engine.rs` — import, `open_recorder`, five `finish_step` sites
- [ ] `htui/tests/chat.rs:756`, `htui/tests/runs_pg.rs:647` — `Unleased`, imports

**T2**
- [ ] `htui-agent/src/record.rs` — `unoffered`, `release_held`, `flush`, `seq_collision`, docs (§3.1-3.2)
- [ ] `htui-agent/tests/recorder.rs` — `appends`, `lose_replies`, three tests (§3.4)
- [ ] `htui-orch/src/fake.rs` — stall triple, `suspend_after_done`, `after_done` (§3.3)
- [ ] `htui-orch/src/conformance.rs` — trait method + impl, case, `CASES`, arm, docs, pin 73
- [ ] `htui-orch/tests/fake_conformance.rs:16` — 73
- [ ] `.claude/plans/mod-40-multi-writer-hardening.plan.md` — R-1 sentence (F-11); MOD-41 note (F-7)

### 6. Gates

Environment for every command: `source /home/user/htui-env.sh` (ORT, `HTUI_TEST_DATABASE_URL=
postgres://htui:htui@localhost:5432/postgres`, `USERNAME=htui-ci`, `RUST_BACKTRACE=0`).

**sqlx offline data (T1 commit 2, once per box for the first three lines):**

```bash
cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,sqlite
PGPASSWORD=htui psql -h localhost -U htui -d postgres -c "CREATE DATABASE htui_sqlx"
DATABASE_URL=postgres://htui:htui@localhost:5432/htui_sqlx cargo sqlx migrate run --source crates/htui-store/migrations
cd crates/htui-store && DATABASE_URL=postgres://htui:htui@localhost:5432/htui_sqlx \
  cargo sqlx prepare -- --all-targets --all-features
```

(`README.md:494-511`: from inside the crate, `--all-targets --all-features` mandatory. README's
`5439`/`postgres` DSN is another box's.) Expected: `git status crates/htui-store/.sqlx` shows three
deletions and three additions; `ls crates/htui-store/.sqlx | wc -l` is 281.

**T1:**

```bash
cargo fmt --all -- --check
cargo build --workspace --all-features --all-targets
cargo test -p htui-core  --all-features -- --test-threads=2
cargo test -p htui-store --all-features -- --test-threads=2
cargo test -p htui-agent --all-features -- --test-threads=2
cargo test -p htui-orch  --all-features -- --test-threads=2
cargo test -p htui --all-features --test chat --test runs_pg --test chat_usage_pg -- --test-threads=2
cargo clippy --workspace --all-features --all-targets -- -D warnings
(cd crates/htui-store && DATABASE_URL=postgres://htui:htui@localhost:5432/htui_sqlx \
  cargo sqlx prepare --check -- --all-targets --all-features)
```

**T2:**

```bash
cargo fmt --all -- --check
cargo test -p htui-agent --all-features -- --test-threads=2
cargo test -p htui-orch  --all-features -- --test-threads=2
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=2
```

The last is the milestone's close; `every_provider_failure_leaves_a_valid_prompt` fails on main
already (plan Validation) and is the only accepted failure.
