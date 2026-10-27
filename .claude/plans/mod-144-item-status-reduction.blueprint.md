# Blueprint: MOD-144 — item status reduced to open, blocked, closed

**Plan**: `.claude/plans/mod-144-item-status-reduction.plan.md` (CONFIRMED 2026-10-27; Decisions A, B, C
as recommended). **Design**: `docs/ANA-35.md` §6, §7.1, §7.10.
No source file is written by this blueprint. Line anchors are from `hr/MOD-144` at `d273067f`.
Tasks: T1 (htui-core) → {T2 (htui-store) ∥ T3 (htui-orch)} → {T3b (htui-worker) ∥ T4 (htui-mcp)} → T5
(htui) → T6 (docs, gate). Every commit message ends with
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Plan deviations and gaps (read first)

| # | Plan says / omits | Finding (evidence) | Adjustment (binding for implementers) |
|---|---|---|---|
| PD1 | T3: `enqueue_with` (`engine.rs:724`) `Open \| Failed` → `Open` | The gate keeps the tag check and rung 4 to items `create_run` would accept (doc `:720-723`, test `enqueue_checks_tags_only_where_create_run_would` `:14820`). After MOD-144 an `open` item may hold a **live** run; `refuse_missing_tags` (`:890`) / `refuse_rung_four` (`:867`) would then CAS `open → blocked` **successfully** and block an item whose run is walking (today `queued`/`in_progress` made that CAS `Ok(false)`) | Replace the `matches!` gate with `htui_core::store::startable(item.status, live)?` first (T3 below); the tag check then runs unconditionally. `create_run` stays the authority |
| PD2 | Decision B: "mismatch **or missing** stamp → drop + recreate" | `qdrant_live.rs:420 an_unrecorded_collection_is_stamped_once` and `:441 stamping_keeps_the_collections_other_metadata` plant embedder-less collections with foreign metadata; a drop on "missing" erases them, and a metadata-less Qdrant (`UNSTAMPED_WARNED`, `vector.rs:831`) would drop on every connect | Drop only when the collection records **this** embedder and its payload stamp is missing or different. An embedder-less collection is stamped with both keys, never dropped. `ExistingCollection::open` never drops |
| PD3 | Pins: `Pending(22)` at `migrations.rs:3561,3662,3671,3694` only | Also `tests/connect.rs:140,156,242` (`Pending(22)`/`pending, 22`), the applied-version list `migrations.rs:118-129`, and the `_sqlx_migrations` count `migrations.rs:3845` | All move with the migration (T2 C1) |
| PD4 | T5 file list omits `ui/tabs/backlog/detail/runs.rs`; `closeout::Preview` not mentioned | The picker filters by `closes_from(preview.status)` (`runs.rs:1090-1112, 1140`); `Preview` carries `status: Status` only (`closeout.rs:24-25`) | `Preview` gains `finished: bool` (T3); `closeout::preview` gains `finished`; T5 updates `runs.rs`. Hard coupling: htui does not build without both |
| PD5 | "Conformance covers each normative rule on Mem and Pg" | `ready_items`, `queue_rows`, `prune_finished_entries`, `close_batch`, `missing_tags*` are **inherent** on `MemStore` (`mem.rs:830, 851, 1080, 1179, 1258`), not trait methods, so `conformance.rs` cannot reach them | Paired tests: Mem unit tests (T1) + Pg-vs-Mem parity tests in `pg_criteria.rs` (T2) with the same rows. Readiness is still in conformance through `items(ready)` |
| PD6 | "`queue_rows`' LATERAL keeps cancelled runs (`CancelledInBatch` reads them)" | `Wait::CancelledInBatch` reads `LiveFacts.cancelled` (`batch_cancelled_items`), not `QueueRow.latest_run` (`queue.rs:518-520`; `mem.rs:1294-1306`). The cancelled `latest_run` is what `a_settled_live_run_does_not_shadow_the_item` (`queue.rs:1279`) and `QueueRunFact` read | Keep the shape (two latest runs); fix the rationale in docs. `Escalation::Failed` takes `run`/`failure` from `latest_run` only when that run **is** `failed` |
| PD7 | Cache: "`items(ready)`, `waiting_candidates`, `item_summary_of`, link nodes" | `READ_CASES` `upstream_in_scope_no_summary` and `coverage_carries_status_and_resolution` (`conformance.rs:952, 960`) also run against `CacheStore` | Cache `upstream_summaries` (`cache/read.rs:882`) and `requirement_coverage` (`:1270`) fill `latest_run_status` too |
| PD8 | — | `resolution_not_closable(item, status: Status, resolution)` (`traits.rs:3118`) names the stored status; the law now keys on the run | Signature `(item: ItemId, state: ItemState, resolution: Resolution)`; Mem and Pg pass `ItemState::of(status, latest)` |
| PD9 | `NotBlocked`/`NotClosable` "carry `ItemState`" | `unblock_enabled(item, runs)` is fed **active runs only** (`engine.rs:2509-2523`, `views.rs:470-480`); with no live run the state cannot tell `done`/`failed` from `open` | Callers pass **every** run (resumable `false` for finished ones); the case logic still filters live runs. The waiting-list path (`WaitingCandidate.runs` is live-only) discards the text (`views.rs:614`) |
| PD10 | — | Tests build two live graph runs on one item by walking `queued → open`: `pg_criteria.rs:5026 finish_run_holds_the_item_while_another_run_is_live`, conformance `finish_run_moves_run_and_item_together` leg 2 (`conformance.rs:14714`) | Both premises become illegal: delete/rewrite (T1, T2) |
| PD11 | Fixtures: "every `ItemState` appears" in htui | htui has 8 demo items, `ItemState` has 9 (`fixtures.rs:946-1112`); conformance relies on `ids::BOX` having two free slots (`box_.rs:341`; `conformance.rs:17108-17150`) | `every_item_state_appears_in_the_demo` (whole demo); `stopped` on Vulkan FEAT-1. Every new **live** demo run has `executing_box_id: None` and no lease |
| PD12 | T2 lists `pg/demo.rs` | `demo.rs:414-445` inserts `row.status.as_str()` verbatim | No edit; T1's fixture must satisfy `chk_item_status` and `uq_run_item_live` |
| PD13 | — | `engine.rs:14715 enqueue_refuses_missing_tags_on_a_failed_item_and_leaves_it_failed` pins "a failed item cannot be blocked" | Intended change (ANA-35 §7.1: blocking writes are `open → blocked`): rewrite as `…_and_blocks_it` |
| PD14 | Decision A: "MOD-125's run/step snapshots move with it" | No `.snap` in any crate contains `⊘`; the `STOPPED` glyph is pinned only by `label.rs:530, 546` | No snapshot moves for `■`; any snapshot showing `⊘ → ■` is a regression. `CANCELLED` (`label.rs:84`) also uses `⊘` — out of scope, flag to the maintainer |
| PD15 | T6: CONCEPTS "if it states the eight statuses" | `CONCEPTS.md:51-52` and `docs/REQUIREMENTS.md:272-283` (R-ENT-8, amended 2026-10-18) already state the three-status model | T6 edits README and HANDOFF only |
| PD16 | T5: Body renders the `ItemState` | Body gets `StoreReply::Item` (`Item`, no run fact; `body.rs:99-103`) | Body also keeps the `Runs` reply and derives the state with the core helper (B4) |
| PD17 | — | `htui/src/store_worker.rs::ready_here` is a readiness consumer in the binary (tests `:3973-4067`) | Follows `ItemFilter`; only test literals move (T5) |
| PD18 | Close-out law: "`rejected`/… close `open` or `blocked`"; `done`/`concluded` "need a finished item" | Silent on blocked + finished | Architect's reading (B2): all six need `open \| blocked`; `done`/`concluded` additionally need `finished`. A blocked finished item may close `done`. Pinned by `close_out_law_reads_the_latest_graph_run` and `a_blocked_finished_preview_offers_all_six` |

## Blueprint decisions (on top of the plan's rules)

- **B1 — `ItemState` text.** `str_enum!` texts `needs_you, failed, running, stopped, blocked, queued, open,
  done, closed` (ALL = attention order). `as_str` equals `RunStatus::as_str` for queued, running,
  stopped, done, failed, so SQL derives the state with one `CASE`. `ItemState::word()` is the redesign
  word (`needs you` with a space, else `as_str`): prompt upstream, kit marks, refusal sentences, filter
  summary all print `word()`.
- **B2 — close-out law.** `closes_from(self, status, finished)`: `status ∈ {open, blocked}` and
  (`finished` or not `done`/`concluded`). `default_for(finished_output_kind: Option<&str>) -> Resolution`:
  `None` → `withdrawn`, `Some("verdict")` → `concluded`, `Some(_)` → `done` (`Some("")` for a finished
  run with no readable snapshot). A `closed` item is refused by the caller first (`NotClosable`).
- **B3 — one start rule.** `htui_core::store::startable(status, live: Option<RunStatus>)`: `Ok` iff
  `open` and no live graph run, else `Constraint(illegal_move("item", ItemState::of(status, live),
  "queued"))` — today's sentence shape (`` item.status `running` cannot move to `queued` (ANA-2 §4.3) ``).
  Called by Mem/Pg `create_run` under the item lock, by `start_enabled`, and by `enqueue_with` (PD1).
- **B4 — one latest-run rule per store.** Rust: `latest_graph_run` (generic over `Run` and `RunSummary`,
  below). Pg: the `lr` LATERAL. Cache: `LATEST_OF_I`/`LATEST_OF_T`. Mem: `State::latest_graph_run_of`.
  All are "`kind = 'graph'`, `status <> 'cancelled'`, `ORDER BY queued_at DESC, id DESC LIMIT 1`".
- **B5 — live set.** `queued, running, awaiting_approval, stopped` (`RunStatus::is_active`) everywhere a
  live graph run is meant: `uq_run_item_live`, `create_run`, `startable`'s callers, `close_out`'s probe.
  The claim slot count stays `running` only.
- **B6 — 0023 writes no `COMMENT ON`.** So `the_ana_column_comments_are_present_and_verbatim` and its
  count pins do not move.
- **B7 — `closed_at`.** Only `close_out` writes it. `transition` (Mem `mem.rs:2244`, Pg `write.rs:1923`)
  stops touching it; 0023 clears it on every row it opens or blocks.

## Build matrix (the workspace is red from T1 until T5)

| After | Builds green (`-p`) | Red |
|---|---|---|
| T1 | htui-core | store, orch, worker, mcp, htui |
| T2 + T3 | + htui-store, htui-orch | worker, mcp, htui |
| T3b + T4 | + htui-worker, htui-mcp | htui |
| T5 | workspace | — |

Never run `cargo build --workspace`/`cargo test --workspace` before T5. `cargo sqlx prepare` compiles
only htui-store + htui-core, so T2 can regenerate `.sqlx` while orch is red.

---

## Migration `crates/htui-store/migrations/0023_item_status.sql` (full text)

```sql
-- 0023_item_status.sql - MOD-144 (ANA-35 §7.1, §7.10 row 1). Forward-only (R-STO-5).
--
-- item.status shrinks to open | blocked | closed. Progress (queued, running, needs you, stopped,
-- failed, done) is derived from the item's latest graph run: newest kind = 'graph' run by
-- (queued_at, id), cancelled skipped. "One live graph run per item" moves from the item-status
-- lock to create_run's explicit check plus uq_run_item_live; live includes 'stopped' (MOD-125).
--
-- Order: (1) refuse readably when an item already has two live graph runs, naming the items, so the
-- unique index never fails with a bare 23505; (2) uq_run_item_live; (3) the §7.1 mapping, with the
-- updated_at trigger off as 0006 does (the mirror rebuilds on schema_version 23 regardless): a done
-- or failed item whose latest graph run does not say so is "unexplained" -- done -> closed/done with
-- a note, failed -> blocked with a note; every other removed value -> open; closed_at is cleared on
-- every row that is not closed (only close_out stamps it from now on); (4) the CHECK swap.
-- Notes are written as the item's created_by (item_note.created_by is NOT NULL). No COMMENT ON (the
-- ANA column comment pins stay), no new table (TABLES stays 51), no cache migration (cache
-- item.status has no CHECK; the schema_version bump rebuilds every mirror). A headless worker never
-- migrates: migrate from a TUI first.

-- --------------------------------------------------------------------------------------------
-- 1. Pre-check: at most one live graph run per item
-- --------------------------------------------------------------------------------------------

DO $$
DECLARE
    offenders TEXT;
BEGIN
    SELECT string_agg(p.slug || ':' || i.key, ', ' ORDER BY p.slug, i.key_prefix, i.key_number)
      INTO offenders
      FROM item i
      JOIN project p ON p.id = i.project_id
     WHERE (SELECT count(*)
              FROM run r
             WHERE r.item_id = i.id
               AND r.kind = 'graph'
               AND r.status IN ('queued','running','awaiting_approval','stopped')) > 1;
    IF offenders IS NOT NULL THEN
        RAISE EXCEPTION 'MOD-144 (0023_item_status): an item may have one live graph run, and these have more: %. Cancel all but one live run of each with the previous htui, then start this one again.', offenders
            USING ERRCODE = 'check_violation';
    END IF;
END
$$;

-- --------------------------------------------------------------------------------------------
-- 2. Expand: the declarative guard
-- --------------------------------------------------------------------------------------------

CREATE UNIQUE INDEX uq_run_item_live ON run (item_id)
    WHERE kind = 'graph' AND status IN ('queued','running','awaiting_approval','stopped');

-- --------------------------------------------------------------------------------------------
-- 3. Migrate: the §7.1 mapping
-- --------------------------------------------------------------------------------------------

CREATE TEMP TABLE mod144_unexplained AS
SELECT i.id, i.status, i.created_by
  FROM item i
  LEFT JOIN LATERAL (
        SELECT r.status
          FROM run r
         WHERE r.item_id = i.id
           AND r.kind = 'graph'
           AND r.status <> 'cancelled'
         ORDER BY r.queued_at DESC, r.id DESC
         LIMIT 1
       ) lr ON true
 WHERE i.status IN ('done','failed')
   AND lr.status IS DISTINCT FROM i.status;

INSERT INTO item_note (item_id, body, created_by)
SELECT u.id,
       CASE u.status
           WHEN 'done' THEN 'closed by the status migration: marked done with no finished run'
           ELSE 'failed outside a run before the status migration; clear to run it again'
       END,
       u.created_by
  FROM mod144_unexplained u;

ALTER TABLE item DISABLE TRIGGER trg_item_updated_at;

UPDATE item i
   SET status = 'closed', resolution = 'done', closed_at = COALESCE(i.closed_at, now())
  FROM mod144_unexplained u
 WHERE u.id = i.id AND u.status = 'done';

UPDATE item i
   SET status = 'blocked', closed_at = NULL
  FROM mod144_unexplained u
 WHERE u.id = i.id AND u.status = 'failed';

UPDATE item
   SET status = 'open', closed_at = NULL
 WHERE status IN ('queued','in_progress','awaiting_approval','done','failed');

UPDATE item
   SET closed_at = NULL
 WHERE status IN ('open','blocked') AND closed_at IS NOT NULL;

ALTER TABLE item ENABLE TRIGGER trg_item_updated_at;

DROP TABLE mod144_unexplained;

-- --------------------------------------------------------------------------------------------
-- 4. Contract: the three-value CHECK (0001's unnamed CHECK is item_status_check)
-- --------------------------------------------------------------------------------------------

ALTER TABLE item DROP CONSTRAINT item_status_check;
ALTER TABLE item ADD CONSTRAINT chk_item_status
    CHECK (status IN ('open','blocked','closed')) NOT VALID;
ALTER TABLE item VALIDATE CONSTRAINT chk_item_status;
```

Notes for the implementer:
- `DO $$` has precedent (`0001_init.sql:574`); sqlx runs each migration in one transaction (plan claim
  20), so a raised exception leaves `_sqlx_migrations` at 22 and nothing applied.
- The keys must be in the `RAISE` **message**: `PgDatabaseError`'s Display prints the message only, and
  `MigrateError::ExecuteMigration` wraps it as "while executing migration 23: …". No `DETAIL`/`HINT`.
- `chk_item_resolution_iff_closed` (0006) holds: the `closed` mapping sets `resolution = 'done'` in the
  same statement; no other row gains or loses `closed`.
- A `done` item whose latest non-cancelled graph run is **live** would be closed by rule 3a. It is
  unreachable through the store (a live run needs `done → open → queued` first, which leaves the item
  `queued`/`in_progress`), so it is not special-cased; the migration test does not plant it.
- `idx_run_item ON run(item_id, queued_at DESC)` (`0001_init.sql:465`) serves the LATERAL.

---

## T1 — htui-core: model, MemStore, conformance (serial, first)

### T1.1 Model signatures

**`src/model/item.rs`**
- `Status` (`:8-28`) → three variants in DDL order:
  ```rust
  str_enum!(
      /// `item.status` (MOD-144, ANA-35 §7.1): what a person decided. Progress is the latest
      /// graph run's, derived by [`ItemState::of`].
      Status {
          /// Not blocked and not closed; its runs say how far it got.
          Open => "open",
          /// A human must clear this, with a note (ANA-2).
          Blocked => "blocked",
          /// Closed by close-out, with a resolution.
          Closed => "closed",
      }
  );
  ```
- `impl Status` (`:30-67`): delete `is_terminal` (`:30-36`). `can_move_to` becomes
  `matches!((self, to), (Self::Open, Self::Blocked) | (Self::Blocked, Self::Open))`. Rewrite its doc:
  open ↔ blocked only; `closed` only by close-out; drop the D161 `blocked → awaiting_approval` paragraph.
- New `ItemState` after `Status` (import `crate::model::run::RunStatus`):
  ```rust
  str_enum!(
      /// The display state of an item (MOD-144, ANA-35 §7.1): derived, never stored. `ALL` is the
      /// attention order the Backlog sorts and groups by.
      ItemState {
          NeedsYou => "needs_you", Failed => "failed", Running => "running", Stopped => "stopped",
          Blocked => "blocked", Queued => "queued", Open => "open", Done => "done",
          Closed => "closed",
      }
  );
  impl ItemState {
      /// `closed`, `blocked`, else the latest non-cancelled graph run's state, else `open`.
      #[must_use]
      pub const fn of(status: Status, latest: Option<RunStatus>) -> Self {
          match (status, latest) {
              (Status::Closed, _) => Self::Closed,
              (Status::Blocked, _) => Self::Blocked,
              (Status::Open, Some(RunStatus::AwaitingApproval)) => Self::NeedsYou,
              (Status::Open, Some(RunStatus::Failed)) => Self::Failed,
              (Status::Open, Some(RunStatus::Running)) => Self::Running,
              (Status::Open, Some(RunStatus::Stopped)) => Self::Stopped,
              (Status::Open, Some(RunStatus::Queued)) => Self::Queued,
              (Status::Open, Some(RunStatus::Done)) => Self::Done,
              // "Latest" skips cancelled runs; a caller passing one gets what a cancel leaves.
              (Status::Open, Some(RunStatus::Cancelled) | None) => Self::Open,
          }
      }
      /// The redesign's word: `needs you` (a space); every other state its `as_str`.
      #[must_use]
      pub const fn word(self) -> &'static str {
          match self { Self::NeedsYou => "needs you", other => other.as_str() }
      }
      /// Finished: the latest graph run is `done` (MOD-133 adds "and landed").
      #[must_use]
      pub const fn is_finished(self) -> bool { matches!(self, Self::Done) }
  }
  ```
- `Resolution` impl (`:89-114`):
  ```rust
  /// ANA-35 §7.1's close-out law (replaces MOD-38 D1): every resolution closes an `open` or
  /// `blocked` item; `done` and `concluded` also need it `finished` (latest graph run `done`).
  /// The live-run refusal is the caller's and comes first.
  #[must_use]
  pub const fn closes_from(self, status: Status, finished: bool) -> bool {
      matches!(status, Status::Open | Status::Blocked)
          && (finished || !matches!(self, Self::Done | Self::Concluded))
  }
  /// The picker's start (MOD-39 P13, ANA-35 §7.1): `None` = nothing finished → `withdrawn`;
  /// `Some("verdict")` → `concluded`; any other finished kind (`""` for an unreadable snapshot)
  /// → `done`. A `closed` item is refused by the caller first.
  #[must_use]
  pub fn default_for(finished_output_kind: Option<&str>) -> Self {
      match finished_output_kind {
          None => Self::Withdrawn,
          Some("verdict") => Self::Concluded,
          Some(_) => Self::Done,
      }
  }
  ```
- `Item::summary` (`:159-178`): add `latest_run_status: None`; doc "the item row alone; a store fills
  `latest_run_status`".
- `ItemSummary` (`:181-210`), appended after `touched_paths`:
  ```rust
  /// MOD-144: the status of the item's latest `kind = 'graph'` run that is not `cancelled`, by
  /// `(queued_at, id)`; `None` when it has none. Appended, never inserted (plan D10).
  #[serde(default)]
  pub latest_run_status: Option<RunStatus>,
  ```
  plus `impl ItemSummary { #[must_use] pub const fn state(&self) -> ItemState { ItemState::of(self.status, self.latest_run_status) } }`.
- `ItemFilter` (`:212-230`): `statuses` doc "the stored value (MOD-11)"; `ready` doc "`open`, no
  non-cancelled graph run, every live `blocked_by` target released (closed, or latest graph run
  `done`)"; append `pub states: Option<Vec<ItemState>>` ("keep items whose derived [`ItemState`] is in
  this list; `None` = any").

**`src/model/run.rs`**
- `RunStatus::Stopped` doc (`:41-44`): "its item stays `in_progress`" → "its item reads `stopped` (MOD-144)".
- After `impl RunStatus` (~`:98`), the one Rust latest-run rule, generic so `Run` (orch, Mem) and
  `RunSummary` (`ReadStore::runs`; TUI Body, orch fake, close-out preview) share it:
  ```rust
  /// What [`latest_graph_run`] orders by; implemented by [`Run`] and [`RunSummary`].
  pub trait RunFacts {
      /// `run.kind`.
      fn run_kind(&self) -> RunKind;
      /// `run.status`.
      fn run_status(&self) -> RunStatus;
      /// `(run.queued_at, run.id)`, the order every store's `ORDER BY` uses.
      fn run_order(&self) -> (DateTime<Utc>, RunId);
  }
  impl RunFacts for Run { /* self.kind, self.status, (self.queued_at, self.id) */ }
  impl RunFacts for RunSummary { /* same fields */ }

  /// MOD-144: the item's latest graph run, newest `kind = 'graph'` and not `cancelled` by
  /// `(queued_at, id)` (ANA-35 §7.1). Every store's SQL repeats this rule.
  pub fn latest_graph_run<'a, R: RunFacts + 'a>(runs: impl IntoIterator<Item = &'a R>) -> Option<&'a R> {
      runs.into_iter()
          .filter(|run| run.run_kind() == RunKind::Graph && run.run_status() != RunStatus::Cancelled)
          .max_by_key(|run| run.run_order())
  }
  ```
  Re-export `RunFacts` and `latest_graph_run` from `model/mod.rs` (`:189`). (`RunId` derives `Ord`,
  `ids.rs:20`; UUIDv7 byte order equals Pg's `uuid` order.)
- `impl GraphSnapshot` (`:542-566`):
  ```rust
  /// MOD-144 (ANA-35 §7.1 `default_for`): the `output_kind` of the snapshot's highest-`position`
  /// phase, read from `phases[]` alone so any `v` decodes; `None` without phases.
  #[must_use]
  pub fn last_output_kind(snapshot: &serde_json::Value) -> Option<String> {
      snapshot.get("phases")?.as_array()?.iter()
          .filter_map(|p| Some((p.get("position")?.as_i64()?, p.get("output_kind")?.as_str()?)))
          .max_by_key(|(position, _)| *position)
          .map(|(_, kind)| kind.to_owned())
  }
  ```

**`src/model/queue.rs`**
- `QueueRow` (`:229-249`): `latest_run` doc "latest graph run, **cancelled included**"; append
  `pub latest_run_status: Option<RunStatus>` ("MOD-144: the latest **non-cancelled** graph run's
  status") and `pub const fn state(&self) -> ItemState`. `open_blockers` doc (`:247`) "targets that are
  not released".
- `Wait::NotReady(Status)` (`:283`) → `NotReady(ItemState)`; Display (~`:553`)
  `write!(f, "not ready: item is {}", state.word())`.
- `classify_entry` doc list (`:428-449`): rule 3 "an item whose state is `failed`"; rules 6-7 "an `open`
  item whose latest run is parked (`needs you`)"; rule 11 carries the `ItemState`. Body (`:470-498`):
  ```rust
  let state = row.state();
  let escalation = match (row.status, state, parked) {
      (_, ItemState::Failed, _) => {
          let failed = latest.filter(|run| run.status == RunStatus::Failed);
          Some(Escalation::Failed { run: failed.map(|r| r.id), failure: failed.and_then(|r| r.failure.clone()) })
      }
      (Status::Blocked, _, Some(run)) => Some(Escalation::ReviewLoopExhausted { run: run.id, note: note() }),
      (Status::Blocked, _, None) => Some(Escalation::Blocked { run: latest.map(|r| r.id), note: note() }),
      (Status::Open, ItemState::NeedsYou, Some(run)) => Some(match run.parked_step { /* unchanged */ }),
      _ => /* missing tags, unchanged */,
  };
  // …
  if !live.ready.contains(&item) { return EntryState::Waiting(Wait::NotReady(state)); }
  ```
  Rules 1-2 stay `Queued | Running`; a `stopped` latest run falls through to `NotReady(Stopped)`.

**`src/model/link.rs`**: `LinkNode` (`:72-87`) and `UpstreamEntry` (`:128-143`, derives `Eq`) append
`#[serde(default)] pub latest_run_status: Option<RunStatus>` and gain `pub const fn state(&self) ->
ItemState`. `UpstreamEntry.status` doc "one of the eight R-ENT-8 strings" → "the stored status". Test
literals `:183-190`, `:259-275` add the field; `:187` `Status::Done` → `(Open, Some(Done))`.

**`src/model/mod.rs`**: re-export `ItemState` (`:133`), `RunFacts`, `latest_graph_run` (`:189`).
`status_matches_check_list` (~`:280-295`) pins `["open","blocked","closed"]`. Delete
`status_terminality_matches_readiness_rule` (`:465-473`).

**`src/model/waiting.rs:17-19`**: doc "an item that is `blocked`, or that owns a run at `awaiting_approval`".

### T1.2 Store trait (`src/store/traits.rs`)

Doc rewrites (each loses every **item** `in_progress`/`queued`/`awaiting_approval`/`done`/`failed`):
`waiting_candidates` (`:305-312`: "`blocked`, or owns a run at `awaiting_approval`; its live runs
`queued | running | awaiting_approval | stopped`"), `transition` (`:366-373`: "open ↔ blocked only; never
touches `closed_at`"), `create_run` (`:1384-1393`: "inserts the run; **writes nothing to the item**;
refused unless the item is `open` with no live graph run ([`startable`]), under the item lock"),
`claim_run` (`:1420-1426`: drop "item `queued -> in_progress`"; MissingTags "its item `open -> blocked`
when open"), `promote_step` (`:1782-1786`) and `park_step` (`:1818-1822`): drop the item clause and D17's
paragraph; `finish_run` (`:1842-1874`): "ends a graph run; the item is not written (MOD-144: its state
derives from the run)", delete the table; `close_out` (`:1876-1898`): "law `closes_from(status,
finished)`; refused while a run is `queued | running | awaiting_approval | stopped`";
`ParkOutcome::Parked` (`:3440`): "The step and its run are `awaiting_approval`".

- Delete `finish_run_item_mirror` (`:3296-3317`) and its imports (`mem.rs:73`, `pg/write.rs:67`).
- `resolution_not_closable` (`:3118`) → `(item: ItemId, state: ItemState, resolution: Resolution)` (PD8).
- New, beside `legal_move` (`:2517`):
  ```rust
  /// MOD-144 Δ2: a live graph run (`RunStatus::is_active`), the set `uq_run_item_live` indexes.
  pub const LIVE_RUN_STATUSES: [RunStatus; 4] =
      [RunStatus::Queued, RunStatus::Running, RunStatus::AwaitingApproval, RunStatus::Stopped];

  /// MOD-144 Δ8: `create_run`'s check under the item lock, shared with `start_enabled` and
  /// `enqueue_with`: an `open` item with no live graph run. `live` is the newest live graph run's
  /// status.
  ///
  /// # Errors
  /// `StoreError::Constraint` with `illegal_move("item", ItemState::of(status, live), "queued")`.
  pub fn startable(status: Status, live: Option<RunStatus>) -> Result<()> {
      if matches!(status, Status::Open) && live.is_none() { return Ok(()); }
      Err(crate::store::StoreError::Constraint(illegal_move("item", ItemState::of(status, live), "queued")))
  }
  ```
  `TransitionLaw for Status` (`:2478-2484`) stays. Check that `LIVE_RUN_STATUSES` does not clash with
  `htui-store/src/pg/read.rs:56`'s test-only const of the same name (different crate; fine).

### T1.3 MemStore (`src/store/mem.rs`)

New `State` helpers near `latest_queue_run` (`:5528`):
```rust
fn latest_graph_run_of(&self, item: ItemId) -> Option<&Run> {
    latest_graph_run(self.runs.values().filter(|run| run.item_id == Some(item)))
}
fn latest_run_status(&self, item: ItemId) -> Option<RunStatus> {
    self.latest_graph_run_of(item).map(|run| run.status)
}
/// The newest live graph run (Δ2: stopped included), the one `uq_run_item_live` allows.
fn live_graph_run(&self, item: ItemId) -> Option<&Run> {
    self.runs.values()
        .filter(|r| r.item_id == Some(item) && r.kind == RunKind::Graph && r.status.is_active())
        .max_by_key(|r| (r.queued_at, r.id))
}
/// Released (ANA-35 §7.1): closed, or latest graph run `done`.
fn is_released(&self, target: &Item) -> bool {
    target.status == Status::Closed || self.latest_run_status(target.id) == Some(RunStatus::Done)
}
fn summary_of(&self, item: &Item) -> ItemSummary {
    ItemSummary { latest_run_status: self.latest_run_status(item.id), ..item.summary() }
}
```

| Method (anchor) | Change |
|---|---|
| `is_ready` `:1590-1603` | `item.status == Open && self.latest_graph_run_of(item.id).is_none() && !links.any(live blocked_by from item whose target is !self.is_released(target))` |
| `matches` `:1605-1638` | after `statuses`: `if filter.states.as_ref().is_some_and(\|s\| !s.contains(&ItemState::of(item.status, self.latest_run_status(item.id)))) { return false; }` |
| `item_summaries` `:1662` | `.map(\|(_, item)\| self.summary_of(item))` |
| `link_node` `:1666-1676`, `upstream_entry` `:1794-1818` | `latest_run_status: self.latest_run_status(item.id)` |
| `waiting_candidates` `:2025-2058` | item filter `item.status == Status::Blocked \|\| parked.contains(&item.id)`; doc too |
| `transition` `:2210-2246` | delete `item.closed_at = to.is_terminal().then_some(now);` and its comment (`:2242-2244`); doc "never touches `closed_at`; only close-out sets it" |
| `queue_rows` `:1080-1104` | add `latest_run_status: state.latest_run_status(item.id)`; `latest_run` stays `latest_queue_run` (cancelled kept, PD6) |
| `open_blocker_keys` `:5553-5568` | `.filter(\|t\| !self.is_released(t))`; doc "not released" |
| `prune_finished_entries` `:1255-1270` | collect `finished: BTreeSet<ItemId>` first (`status == Closed \|\| latest_run_status == Some(Done)`), then `retain(\|item, e\| e.box_id != box_id \|\| !finished.contains(item))`; doc "closed or finished" |
| `close_batch` doc `:1170-1177` | "its item goes back to `open`" → "the item's state falls back to its previous run" (code unchanged) |
| `create_run` `:5747-5850` | below |
| `claim_run` MissingTags `:5897-5907` | `status == Status::Open` → `self.transition(item, Status::Open, Status::Blocked, now)?` |
| `claim_run` Admitted `:5972-5981` | delete the item block |
| `promote_step` `:6944-6982` | delete `item_id` and the item block (`:6974-6981`); doc `:6944` "the step and its run" |
| `park_step` `:6984-7044` | delete `move_item` (`:7009-7020`) and its write (`:7030-7036`), doc `:6986-6992`; three checks then two moves |
| `finish_run` `:7062-7128` | delete everything after the run write (`:7112-7127`); keep the chat note as a comment; doc "the item is not written" |
| `close_out` `:7130-7194` | below |
| `coverage_rows` `:7449` | `item: self.summary_of(item)` |

`create_run`, D14's order kept (lookup, then the law):
```rust
let item = self.require_item(new.item_id)?;
let (status, holder) = (item.status, item.project_id);
startable(status, self.live_graph_run(new.item_id).map(|run| run.status))?;
// … every existing check unchanged …
self.runs.insert(row.id, row.clone());
if let Some(batch) = new.batch_id { self.run_batches.insert(row.id, batch); }
Ok(row)                       // the `self.transition(…Queued…)` at :5848 is deleted
```
Doc `:5746`: "The `run` row alone; the item is not written (MOD-144)".

`close_out` (guard order kept: NotFound, live run, summary kind, summary item, law, commits):
```rust
let status = self.require_item(item)?.status;
/* live-run refusal unchanged */  /* summary checks unchanged */
let latest = self.latest_graph_run_of(item).map(|run| run.status);
if !resolution.closes_from(status, latest == Some(RunStatus::Done)) {
    return Err(StoreError::Constraint(resolution_not_closable(item, ItemState::of(status, latest), resolution)));
}
```

### T1.4 Prompt and fixtures

- `src/prompt/render.rs`: `summary_block` (`:369`) and `one_liner` (`:388`) print `self.state().word()`
  instead of `self.status.as_str()`. Test helper `entry` (`:1037-1046`) adds `latest_run_status: None`.
- `src/prompt/fixtures.rs`: `upstream_entry(..)` (`:65-83`) gains `latest: Option<RunStatus>` after
  `status`. `phase_implement_attempt2`: `:205` `InProgress` → `(Open, Some(Running))`; `:214` `Done` →
  `(Open, Some(Done))`. `phase_oversize`: `:652` → `(Open, Some(Running))`; `:661` → `(Open,
  Some(Queued))`; `:670` → `(Open, Some(Done))`. `Closed` rows pass `None`.
- `src/fixtures.rs`: module doc `:10` "three statuses, nine item states". Add `RUN_4..RUN_8 =
  (class::RUN, 3..7)` to `demo_ids!` (`:289-308`). Every new run: `kind: Graph`, `graph_snapshot:
  Some(demo_graph_snapshot())`; live ones `executing_box_id: None`, no lease, `started_at` set for
  running/parked/stopped (PD11).

| Item (spec line) | Was | Status | Run(s) | ItemState |
|---|---|---|---|---|
| htui ANA-1 `:954` | done | open | RUN_3 done (exists) | done |
| htui ANA-2 | open | open | — | open |
| htui FEAT-1 `:980` | in_progress | open | RUN_1 done + **RUN_4 running**, `queued_at demo_at(2, 9)` (rework: holds FEAT-2) | running |
| htui FEAT-2 | blocked | blocked | — | blocked |
| htui FEAT-3 `:1006` | queued | open | RUN_2 queued (exists) | queued |
| htui FIX-1 | closed | closed | — | closed |
| htui TOOL-1 `:1032` | awaiting_approval | open | **RUN_5 awaiting_approval**, no step (judge park) | needs_you |
| htui CLEAN-1 `:1045` | failed | open | **RUN_6 failed**, `failure: Some("verify failed")`, `finished_at` set | failed |
| agy ANA-1 `:1058` | done | open | **RUN_7 done** (`project_id: PROJECT_AGY`) | done |
| vulkan FEAT-1 `:1097` | in_progress | open | **RUN_8 stopped** (`workspace_id: WORKSPACE_GRAPHICS`) | stopped |

Ready set unchanged: {ANA-2, AGY FEAT-1, AGY FIX-1, VULKAN TOOL-1}. `fixtures.rs:2320` →
`every_item_state_appears_in_the_demo` (for each `ItemState::ALL`, some item's
`ItemState::of(status, latest_graph_run(its runs))` equals it). `fix_1_is_closed_as_done` unchanged.
Before accepting, check that no startup sweep (`adopt_runs`, the demo store's recovery) settles RUN_4,
RUN_5 or RUN_8 — `adopt_runs` filters `executing_box_id == box`, so `None` is safe (verify).

### T1.5 Tests first

**Unit tests**
- `model/item.rs`: `item_state_of_reads_status_then_latest_run` (all 24 rows `Status × ({None} ∪
  RunStatus::ALL)`, transcribed from ANA-35 §7.1, not from the match); `item_state_all_is_the_attention_order`;
  `item_state_word_spells_needs_you`; `the_item_status_table_sanctions_exactly_open_and_blocked` (replaces
  `SANCTIONED`); keep `a_closed_item_reaches_no_other_status`, `nothing_transitions_into_closed`;
  `the_close_out_law_sanctions_exactly_the_ana_35_triples` (`CLOSE_OUT_SANCTIONED: &[(Status, bool,
  Resolution)]` = Open/Blocked × {false,true} × 4 non-success + Open/Blocked × true × {Done, Concluded}
  = **20**); `the_default_resolution_is_a_sanctioned_close_out` (`None→Withdrawn`, `Some("")→Done`,
  `Some("plan")→Done`, `Some("verdict")→Concluded`; each closes `Open` with `finished = kind.is_some()`).
- `model/run.rs`: `last_output_kind_reads_the_highest_position` (out-of-order phases; `None` on `Null`,
  missing `phases`, empty array; a phase without `output_kind` skipped);
  `latest_graph_run_skips_cancelled_and_chat_and_breaks_ties_on_id` (over `Run` and `RunSummary`).
- `store/traits.rs`: `startable_admits_only_an_open_item_with_no_live_run` (errors pinned to
  `illegal_move("item", ItemState::of(..), "queued")` for each `LIVE_RUN_STATUSES` and blocked/closed).
- `model/queue.rs`: `row(status)` → `row(status, latest_run_status)`; `ready()` (`:623`) adds the field.
  `:876` `(Open, Some(Running/Queued))`; `a_live_run_elsewhere_beats_everything` `:905` `(Open,
  Some(Failed))` + a live run elsewhere still `Elsewhere`; `a_failed_item_reads_failed…` `:950` `(Open,
  Some(Failed))`; `:1036`, `:1055` `(Open, Some(AwaitingApproval))`;
  `awaiting_approval_without_a_parked_run_is_not_ready` `:1082` → `a_done_item_is_not_ready` (`(Open,
  Some(Done))` → `NotReady(ItemState::Done)`, `"not ready: item is done"`); `:1102-1132`;
  `a_settled_live_run_does_not_shadow_the_item` `:1280` second leg `(Open, Some(Stopped))` → `"not ready:
  item is stopped"`; new `a_cancel_after_a_failure_still_reads_failed_without_a_run`.
- `store/mem.rs` unit tests: `create_run_moves_the_item_and_writes_nothing_when_the_law_refuses`
  (`:13564-13573`: item stays `open`; refusal is `startable`'s `` item.status `running` cannot move to
  `queued` ``); `:13881`, `:14053` item stays `Open`; `:14434`, `:14621-14646` drop the item legs;
  `:15123`, `:15195` replace `transition(FEAT_1, InProgress, Done)` with `finish_run(RUN_4, Cancelled)`
  (latest is then RUN_1 `done`); `queue_rows_carry_the_latest_graph_run_note_and_open_blockers`
  (`:16813, 16844`) assert `latest_run_status` too. **New** (PD5): `prune_drops_closed_and_finished_entries_only`,
  `ready_items_skip_an_item_with_any_non_cancelled_run`,
  `queue_rows_latest_run_keeps_cancelled_but_latest_run_status_skips_it`,
  `open_blockers_drop_a_target_whose_latest_run_is_done`. T2 mirrors each in Pg.

**Conformance (`src/store/conformance.rs`)** — case names are report keys (`:75` "A name never
changes"): keep names, rewrite bodies and docs; append new cases. 22 `CASES` + 2 `READ_CASES` + the
`parked_run` helper use removed values:

| Case (anchor) | Rewrite |
|---|---|
| `status_cas_keeps_version` `:1513` | open→blocked→open; stale `from` leg `(Open→Blocked)` twice; `closed_at` stays `None` |
| `run_create_moves_the_item` `:5379` | item stays `Open`, `updated_at` unchanged; FEAT-1 (RUN_4 live) refused with `startable`'s sentence; CLEAN-1 (latest failed) may start, `latest_run_status` becomes `Queued` |
| `claim_run_admits_one_and_refuses_the_second` `:5625, 5647` | item stays `Open`; precondition via `ItemState::Queued` |
| `claim_run_applies_the_isolation_and_path_rules` `:5874` | precondition `Open` |
| `park_step_moves_step_run_and_item_or_nothing` `:6359-6494` | `rows()` reads `(step, run, item state)`: `Running` → park → `NeedsYou`; the "item moved first" leg is open→blocked, then park leaves `Blocked` |
| `claim_run_fails_a_run_whose_item_needs_a_tag_the_box_lacks` `:6577, 6702` | MissingTags `Open → Blocked`; the clean claim leaves `Open` |
| `stop_run_is_a_fenced_cas_on_running` `:7549` | item `Open`, state `Stopped` after stop |
| `:7753`, `:7805`, `:7876`, `adopt_runs_settles_…_as_stopped` `:8019` | item `Open` throughout; assert state where it was asserted |
| `parked_run` helper `:8204-8226` | `to: Option<Status>`: `None` parks only the run; `Some(Blocked)` = `transition(item, Open, Blocked)` |
| `step_create_and_transition_law` `:11879` | precondition `Open` |
| `gate_answers_write_their_outcome` `:12113, 12137` | drop item transitions; assert state `NeedsYou` after promote |
| `write_document_allocates_its_version` `:12916` | drop `transition(ANA_2, Queued, Open)` |
| `close_out_refuses_a_live_run` `:13069-13188` | FEAT-3 stays `Open` (state `Queued`); drop the queued→open leg; final leg `finish_run(RUN_4, Cancelled)` then close FEAT-1 `Done` |
| `illegal_transitions_are_constraint` `:13235-13334` | item pairs `[(Open, Blocked), (Blocked, Open)]`; every other target refused; unknown item `(Open, Closed)` → NotFound |
| `close_out_resolution_law` `:13438` | sweep the live-run-free htui items (ANA-1, ANA-2, FEAT-2, FIX-1, CLEAN-1); law `closes_from(status, finished)`; sentence `resolution_not_closable(item, state, r)`; swept states `[Open, Blocked, Failed, Done, Closed]`; doc `:13377` |
| `transition_never_reaches_closed` `:13540` | `illegal_move("item", status, Closed)` for open/blocked/closed; unknown id `(Open, Closed)` |
| `coverage_lists_citing_items_with_resolution` `:14513` | compare `item.state()` |
| `finish_run_moves_run_and_item_together` `:14723` | leg 1: item stays `Open`, `closed_at` None, state `Done`; leg 2 (PD10) → "a second `create_run` on an item with a live run (stopped included) is refused"; leg 3 state `Failed`; leg 4 cancel → `Open`; legs 5-7 item stays `Open` |
| `upstream_in_scope_no_summary` (READ) `:15212` | `entry.state()`: AGY ANA-1 `Done`, TOOL-1 `NeedsYou` |
| `coverage_carries_status_and_resolution` (READ) `:15859, 15886` | `(ANA_1, Done)`, `(FEAT_1, Running)` via `item.state()` |
| `waiting_candidates_hold_every_park_and_nothing_else` `:17008` | gate/selection/judge/reopened stay `Open`; escalation/stopped/queued/mid_walk `Blocked` via `transition(Open, Blocked)`; TOOL-1 carries `(RUN_5, AwaitingApproval, [])`; FEAT-1 absent |
| `withdraw_link_only_what_this_run_proposed` `:20199` | drop `transition(item, Queued, Open)` |
| `no_delete_path`, `filter_status_project_tags_ready` `:1669, 1805-1900` | `statuses: [Open]` lists every open item in scope order; add a `states: [ItemState::Open]` leg (ANA-2, AGY FEAT-1, AGY FIX-1); FEAT-2 blocked by running FEAT-1 (doc `:1876`) |

New `CASES` (append to `CASES` and `run_case_inline`, each `Box::pin`'d):
1. `create_run_refuses_a_second_live_graph_run` — for queued, running, awaiting_approval, stopped: `startable`'s sentence, rows unchanged; after `done`/`failed`, admitted again.
2. `readiness_skips_cancelled_runs` — only-cancelled → ready; `done` or `failed` run → not ready.
3. `a_done_run_releases_its_dependents` — B `blocked_by` A; A's run `done` makes B ready while A is `open`.
4. `a_rework_run_holds_dependents_until_cancelled` — A done; new run on A → B not ready; cancel → B ready.
5. `waiting_candidates_follow_blocked_items_and_parked_runs` — open + parked run listed; open + finished not; blocked without run listed.
6. `close_out_law_reads_the_latest_graph_run` — `done` refused on open-unfinished, allowed finished; cancelled after `done` still allows `done`; blocked+finished closes `done`; failed-latest closes `withdrawn`, refuses `done` naming state `failed`.
7. `a_finished_snapshot_defaults_a_verdict_to_concluded` — run whose snapshot's last phase is `verdict`, finished `done`, read back via `store.run`: `last_output_kind == Some("verdict")`, `default_for` → `Concluded`, closes (Pg JSONB round-trip).
8. `item_state_is_derived_from_the_latest_graph_run` — one minted item per derivation row; checked in `items()`, `links()` nodes, `requirement_coverage` and `states` filter rows.
9. `transition_never_touches_closed_at` — open→blocked→open leaves `closed_at` and `version`; only close-out sets it.

Pin: `crates/htui-core/tests/mem_store.rs:37` **206 → 215** (message: "…and MOD-144's nine for the item
status reduction"). `READ_CASES` stays 15. T2 moves `pg_conformance.rs:43` to the same number.

### T1.6 Snapshots (htui-core)
Move: `tests/snapshots/prompt_render__section_upstream.snap`,
`prompt_render__section_upstream_one_liners_only.snap` (`(in_progress)` → `(running)`),
`prompt_golden__prompt_implement_attempt2.snap` (upstream lines, digest line if carried).
`tests/prompt_digest.rs:444-452` literal adds `latest_run_status: None`. Must not move:
`prompt_render__section_upstream_summaries_only.snap` and everything else. Oversize ladder tests (`:400`)
must stay green unchanged.

### T1.7 Commit groups
1. `feat(MOD-144): ItemState, three-value item status, run-anchored close-out; MemStore stops mirroring runs`
   — model/*, traits.rs, mem.rs, fixtures.rs, prompt/{render,fixtures}.rs, their unit tests (incl. the
   four PD5 Mem tests). One commit, because the model alone does not compile against mem.rs.
   Green: `cargo test -p htui-core --all-features --lib`.
2. `test(MOD-144): conformance rewrites and nine new cases; CASES 215` — conformance.rs, `tests/mem_store.rs:37`.
3. `test(MOD-144): accept the moved prompt snapshots` — after review: only `in_progress → running`.

### T1.8 Verification
```bash
cargo test -p htui-core --all-features
cargo insta test -p htui-core --all-features --review   # only T1.6's snapshots may move
cargo clippy -p htui-core --all-features -- -D warnings
cargo clippy -p htui-core -- -D warnings                # featureless
```

---

## T2 — htui-store: migration, Pg, cache, search, `.sqlx` (after T1; parallel with T3)

### T2.0 Facts that bind T2
- `PgStore::schema_version()` = `MIGRATOR` max version (`pg/mod.rs:724`): 23 once the file exists; every
  box rebuilds its mirror. Required: the migration runs with `trg_item_updated_at` off, so the cache's
  `updated_at` cursor would never see the rewritten statuses.
- Cache `run` mirrors `kind`, `status`, `queued_at INTEGER`, `id TEXT`, index `idx_cache_run_item ON
  run(item_id, queued_at DESC)` (`cache_migrations/0001_mirror.sql:103-110`). `queued_at DESC, id DESC`
  in SQLite equals Pg's order (lowercase hyphenated UUIDv7 text sorts as the UUID bytes).
- `close_out`'s item lookup (`write.rs:8019`) is byte-identical to `finish_run_on`'s (`:1645`): deleting
  the latter keeps that `.sqlx` entry.

### T2.1 Pg writes (`src/pg/write.rs`)
Imports (`:25-70`): drop `finish_run_item_mirror` (`:67`); add `startable`, `ItemState`.

- **`finish_run_on`** (`:1580-1690`; body of `finish_run` `:7964`; called by `close_batch` `:10059`):
  SELECT (`:1593-1601`) drops `item_id AS "item_id: ItemId"` (entry moves; delete the "keeps the
  indentation … `.sqlx` entry stands" comment). Delete `:1642-1687` (item `FOR UPDATE`, sibling live
  count `id <> $2`, mirror `UPDATE`). Body: run read, `to.is_terminal()`/`failure` refusals,
  `legal_move(row.status, to)?`, run `UPDATE` (`:1629-1640`), `Ok(())`. Doc `:1580-1584`: "the run's
  terminal move, the run taken `FOR UPDATE`. The item is not written (MOD-144) … `close_batch` runs it
  for each still-`queued` run of the batch". `finish_run` doc (`:7940-7963`): delete the item-lock
  paragraph; keep `# Errors`.
- **`transition`** (`:1893-1945`):
  ```rust
  let moved = sqlx::query!(
      "UPDATE item SET status = $3 WHERE id = $1 AND status = $2",
      id.as_uuid(), from.as_str(), to.as_str(),
  )
  ```
  Doc: "never bumps `version`, never writes a revision, never touches `closed_at` (MOD-144: only
  `close_out` stamps it). `legal_move` admits `open ↔ blocked` only." Delete the H.12 `closed_at` paragraph.
- **`create_run`** (`:5847-5991`): replace `legal_move(status, Status::Queued)?;` (`:5861`) with
  ```rust
  // MOD-144: "one live graph run per item" is this check under the item's FOR UPDATE, backed by
  // `uq_run_item_live` (0023). The newest live run is named, so the refusal says what holds it.
  let live = sqlx::query_scalar!(
      r#"SELECT status AS "status: RunStatus"
           FROM run
          WHERE item_id = $1 AND kind = 'graph'
            AND status IN ('queued','running','awaiting_approval','stopped')
          ORDER BY queued_at DESC, id DESC
          LIMIT 1"#,
      new.item_id.as_uuid(),
  )
  .fetch_optional(&mut *tx)
  .await
  .map_err(map_sqlx)?;
  startable(item.status, live)?;
  ```
  Order kept: NotFound, `startable`, project check. Delete the item `UPDATE` and its `moved != 1`
  branch (`:5964-5985`). Batch-lock comment `:5904-5909`: "lock order is item, then batch; `close_batch`
  locks batch, then the batch's `queued` runs, and no item since MOD-144". Doc `:5814-5833`: "The `run`
  row, refused while the item is not `open` or has a live graph run (`startable`)"; delete the zero-row
  paragraph; `# Errors` names `startable`'s sentence; a racing insert that slipped the lock would be
  `uq_run_item_live`'s 23505 → `Constraint` via `map_sqlx` (unreachable under the lock).
- **`claim_run`** (`:6036-6250`): MissingTags item UPDATE (`:6139-6147`) `AND status = 'queued'` →
  `AND status = 'open'` (zero-row tolerant; comment "an item no longer `open` (blocked by hand) is left
  as it is"). Admitted branch: delete the item UPDATE and comment (`:6235-6246`).
- **`promote_step`** (`:7671-7742`) / **`park_step`** (`:7816-7883`): SELECTs drop `r.item_id AS
  "item_id: ItemId"`; delete the item UPDATE blocks (`:7729-7738`, `:7870-7879`, byte-identical → one
  `.sqlx` entry goes). Docs: "the step and its run are lifted together; the item's `needs you` is
  derived from the run (MOD-144)"; delete the `closed_at` sentence; `park_step` "The run statement is
  `promote_step`'s".
- **`close_out`** (`:7975-8128`): after the live-run probe (`:8033-8052`, unchanged: any kind, four
  live statuses), before the law:
  ```rust
  // MOD-144: `done`/`concluded` need a finished item: its latest graph run, `cancelled` skipped, is
  // `done`. Read under the item lock; no run can go live meanwhile (`create_run` takes the same lock).
  let latest = sqlx::query_scalar!(
      r#"SELECT status AS "status: RunStatus"
           FROM run
          WHERE item_id = $1 AND kind = 'graph' AND status <> 'cancelled'
          ORDER BY queued_at DESC, id DESC
          LIMIT 1"#,
      item.as_uuid(),
  )
  .fetch_optional(&mut *tx)
  .await
  .map_err(map_sqlx)?;
  if !resolution.closes_from(status, latest == Some(RunStatus::Done)) {
      return Err(StoreError::Constraint(resolution_not_closable(item, ItemState::of(status, latest), resolution)));
  }
  ```
  Closing UPDATE (`:8103-8113`, `closed_at = clock_timestamp()`) unchanged — the only `closed_at`
  writer. Doc `:7980-7983`: the item lock stops a `create_run` between probe and UPDATE (MOD-144).
- **`close_batch`** (`:10006-10065`): no SQL change. Doc `:10009-10016`: "its item is not written: a
  cancelled run is skipped by the latest-run rule (MOD-144)"; lock order "batch row, then run rows".
- **`prune_finished_entries`** (`:10141-10166`):
  ```sql
  WITH doomed AS (
      SELECT e.item_id
        FROM queue_entry e JOIN item i ON i.id = e.item_id
       WHERE e.box_id = $1
         AND (i.status = 'closed'
              OR (SELECT r.status FROM run r
                   WHERE r.item_id = i.id AND r.kind = 'graph' AND r.status <> 'cancelled'
                   ORDER BY r.queued_at DESC, r.id DESC
                   LIMIT 1) = 'done')
       ORDER BY e.item_id
         FOR UPDATE OF e)
  DELETE FROM queue_entry e
   USING doomed d
   WHERE e.item_id = d.item_id
  ```
- Unchanged (verified): `fail_run` (`:7901`), `transition_run`, `close_drained_batch` (`:10118`, lists
  `stopped`), `claim_run`'s overlap set (`:6193`). No other `UPDATE item … status` in `src/`.

### T2.2 Shared Pg fragments (spelled out in each macro; macros need literals)

| Name | Text |
|---|---|
| **LATEST** (projection) | `LEFT JOIN LATERAL (SELECT r.status FROM run r WHERE r.item_id = i.id AND r.kind = 'graph' AND r.status <> 'cancelled' ORDER BY r.queued_at DESC, r.id DESC LIMIT 1) lr ON true`, column `lr.status AS "latest_run_status?: htui_core::model::RunStatus"` (the `?` forces `Option`) |
| **LATEST_OF(t)** | `(SELECT tr.status FROM run tr WHERE tr.item_id = t.id AND tr.kind = 'graph' AND tr.status <> 'cancelled' ORDER BY tr.queued_at DESC, tr.id DESC LIMIT 1)` |
| **LIVE** | `status IN ('queued','running','awaiting_approval','stopped')`. `LIVE_RUN_STATUSES` (`read.rs:56`) stays the test pin; its doc names `create_run`, `close_out`, `uq_run_item_live` |
| **UNRELEASED(i)** | `EXISTS (SELECT 1 FROM item_link l JOIN item t ON t.id = l.to_item_id WHERE l.from_item_id = i.id AND l.kind = 'blocked_by' AND l.deleted_at IS NULL AND t.status <> 'closed' AND LATEST_OF(t) IS DISTINCT FROM 'done')` |
| **READY(i)** | `i.status = 'open' AND lr.status IS NULL AND NOT UNRELEASED(i)` (every term non-null, so `$6 = (…)` stays two-valued) |
| **STATE(i)** | `CASE WHEN i.status = 'closed' THEN 'closed' WHEN i.status = 'blocked' THEN 'blocked' WHEN lr.status IS NULL THEN 'open' WHEN lr.status = 'awaiting_approval' THEN 'needs_you' ELSE lr.status END` (B1) |

New unit test in `read.rs` tests (beside `live_runs_literal_matches_run_status`, `:3640`):
`item_state_case_matches_item_state_of` — the CASE as a Rust table `[(Status, Option<RunStatus>, &str)]`
for every `Status` × non-cancelled `Option<RunStatus>`, each text equal to `ItemState::of(..).as_str()`.

### T2.3 Pg reads (`src/pg/read.rs`, `src/pg/rows.rs`)

**`items`** (`:81-143`), full statement:
```sql
SELECT i.id            AS "id: ItemId",
       i.project_id    AS "project_id: ProjectId",
       i.kind_id       AS "kind_id: htui_core::model::ItemKindId",
       i.key           AS "key!",
       i.key_prefix,
       i.key_number,
       i.title,
       i.status        AS "status: htui_core::model::Status",
       i.priority,
       i.required_tags,
       i.updated_at,
       i.touched_paths,
       lr.status       AS "latest_run_status?: htui_core::model::RunStatus"
  FROM item i
  LEFT JOIN LATERAL (
        SELECT r.status FROM run r
         WHERE r.item_id = i.id AND r.kind = 'graph' AND r.status <> 'cancelled'
         ORDER BY r.queued_at DESC, r.id DESC
         LIMIT 1
       ) lr ON true
 WHERE i.project_id = ANY($1)
   AND ($2::uuid[] IS NULL OR i.project_id = ANY($2))
   AND ($3::text[] IS NULL OR i.status = ANY($3))
   AND ($4::text[] IS NULL OR i.required_tags @> $4)
   -- (the existing `position` comment block stays)
   AND ($5::text IS NULL
        OR position(lower($5::text) in lower(i.key)) > 0
        OR position(lower($5::text) in lower(i.title)) > 0)
   AND ($6::bool IS NULL OR $6 = (
            i.status = 'open'
        AND lr.status IS NULL
        AND NOT EXISTS (
            SELECT 1 FROM item_link l JOIN item t ON t.id = l.to_item_id
             WHERE l.from_item_id = i.id AND l.kind = 'blocked_by'
               AND l.deleted_at IS NULL AND t.status <> 'closed'
               AND (SELECT tr.status FROM run tr
                     WHERE tr.item_id = t.id AND tr.kind = 'graph' AND tr.status <> 'cancelled'
                     ORDER BY tr.queued_at DESC, tr.id DESC
                     LIMIT 1) IS DISTINCT FROM 'done')))
   AND ($7::text[] IS NULL OR (CASE
            WHEN i.status = 'closed'  THEN 'closed'
            WHEN i.status = 'blocked' THEN 'blocked'
            WHEN lr.status IS NULL    THEN 'open'
            WHEN lr.status = 'awaiting_approval' THEN 'needs_you'
            ELSE lr.status END) = ANY($7))
 ORDER BY array_position($1, i.project_id), i.key_prefix, i.key_number
```
Bind `$7` as `filter.states.as_ref().map(|l| l.iter().map(|s| s.as_str().to_owned()).collect::<Vec<_>>())`
then `.as_deref()`. Doc `:81-86`: readiness and `states` per the rules.

**`ready_items`** (`:2138-2196`): same `lr` join and column; WHERE:
```sql
 WHERE i.project_id = ANY($1)
   AND i.status = 'open'
   AND lr.status IS NULL
   AND NOT EXISTS (<UNRELEASED body exactly as in `items`>)
   AND NOT EXISTS (SELECT 1 FROM UNNEST(i.required_tags) t
                    WHERE t <> ALL (COALESCE(b.probed_tags, '{}') || COALESCE(b.declared_tags, '{}')))
```
(every projected `latest_run_status` is `None`). Doc `:2140-2143`: "the readiness conjunct is `items`' own".

**`queue_rows`** (`:2408-2522`): keep `lr` (cancelled **included**, feeds `QueueRunFact`) verbatim; add
```sql
  LEFT JOIN LATERAL (
        SELECT r.status FROM run r
         WHERE r.item_id = i.id AND r.kind = 'graph' AND r.status <> 'cancelled'
         ORDER BY r.queued_at DESC, r.id DESC
         LIMIT 1
       ) ls ON true
```
column `ls.status AS "latest_run_status?: RunStatus"`. The `ob` lateral filter (`:2456`) `AND
l.deleted_at IS NULL AND t.status NOT IN ('done','closed')` → `AND l.deleted_at IS NULL AND t.status <>
'closed' AND (SELECT tr.status … LIMIT 1) IS DISTINCT FROM 'done'`. Mapping (`:2492-2503`) fills
`latest_run_status`. Doc `:2394-2402` tells the two latest runs apart.

**`waiting_candidates`** (`:1266-1300`), predicate `:1295-1297`:
```sql
   AND (i.status = 'blocked'
        OR EXISTS (SELECT 1 FROM run r
                    WHERE r.item_id = i.id AND r.status = 'awaiting_approval'))
```

**`requirement_coverage`** (`:1171-1225`): `lr` join after `JOIN item i`, column, literal (`:1203-1216`)
fills `latest_run_status`. **`links`** (`:180-245`): `lr` after `JOIN project p`, column; `LinkNodeRow`
(`rows.rs:180-215`) gains the field, `into_node` passes it. **`upstream_summaries`** (`:556-627`): same;
`UpstreamRow` (`rows.rs:218-254`) + `into_entry`; doc `:549-555` notes the `?`.
Unchanged: `item()`, `active_runs`, `overlapping_runs`, `batch_cancelled_items`, `batch_runs`, `missing_tags*`.

### T2.4 Cache SQLite (`src/cache/read.rs`, runtime SQL)

Beside `ITEM_SELECT` (`:222`):
```rust
/// MOD-144: the latest graph run's status of the item aliased `i`, `cancelled` skipped, `NULL` when
/// none: `pg/read.rs`' `lr` LATERAL as a correlated subquery (SQLite has no LATERAL). The mirror's
/// `queued_at` is INTEGER and `id` lowercase UUID text, so the order is Postgres' `(queued_at, id)`.
const LATEST_OF_I: &str = "(SELECT r.status FROM run r WHERE r.item_id = i.id AND r.kind = 'graph' \
     AND r.status <> 'cancelled' ORDER BY r.queued_at DESC, r.id DESC LIMIT 1)";
/// The same for a `blocked_by` target aliased `t`.
const LATEST_OF_T: &str = "(SELECT tr.status FROM run tr WHERE tr.item_id = t.id AND tr.kind = 'graph' \
     AND tr.status <> 'cancelled' ORDER BY tr.queued_at DESC, tr.id DESC LIMIT 1)";
```
- **UNRELEASED**: `EXISTS (SELECT 1 FROM item_link l JOIN item t ON t.id = l.to_item_id WHERE
  l.from_item_id = i.id AND l.kind = 'blocked_by' AND t.status <> 'closed' AND {LATEST_OF_T} IS NOT
  'done')` (no `deleted_at`: tombstones are not mirrored; `IS NOT` is SQLite's null-safe `<>`).
- **READY**: `CASE WHEN i.status = 'open' AND {LATEST_OF_I} IS NULL AND NOT {UNRELEASED} THEN 1 ELSE 0 END`.
- **STATE**: `CASE WHEN i.status = 'closed' THEN 'closed' WHEN i.status = 'blocked' THEN 'blocked' ELSE
  COALESCE(CASE {LATEST_OF_I} WHEN 'awaiting_approval' THEN 'needs_you' ELSE {LATEST_OF_I} END, 'open') END`.

Changes: `item_summary_of` (`:187-202`) reads `latest_run_status: get::<Option<RunStatus>>(row,
"latest_run_status")?` (every feeding SELECT projects `{LATEST_OF_I} AS latest_run_status`); `items`
(`:360-450`): SELECT `:367-370` adds the column via `format!`, ready clause `:401-409` → ` AND ? =
({READY})`, after `statuses` add `AND ({STATE}) IN (?, …)` binding `state.as_str()`. Bind order follows
clause order: projects, narrowed, statuses, **states**, tags, text, ready. `links` (`:465-520`),
`upstream_summaries` (`:830-895`, PD7), `requirement_coverage` (`:1270-1295`, `format!` + `AssertSqlSafe`
as `items` does): project the column and fill the field. `waiting_candidates` (`:1337-1346`): `AND
(i.status = 'blocked' OR EXISTS (SELECT 1 FROM run r WHERE r.item_id = i.id AND r.status =
'awaiting_approval'))`. No `ready_items`/`queue_rows` in the cache (`backend.rs:542-548, 693-698`).

### T2.5 Search (`src/vector.rs`, `src/vector_sync.rs`) — Decision B as amended by PD2

- Payload `status` becomes 3-valued with no code change (`Subject::Item.status: Status`, `payload`
  `:540`). Staleness `p.status == Some(summary.status)` (`vector_sync.rs:94`) unchanged; run moves stop
  re-embedding items. An old point carrying `in_progress` fails `parsed(payload, STATUS)` in
  `indexed_point` (`:602-611`); its section points would be orphaned — hence the drop.
- New:
  ```rust
  /// The collection metadata key the payload layout's version is stored under (MOD-144 Decision B).
  pub const PAYLOAD_SCHEMA_KEY: &str = "payload_schema";
  /// The payload layout this htui writes. 2: MOD-144's three-value `status`. Bump it with any change
  /// to what a point's payload means; a collection stamped otherwise is dropped, recreated and
  /// refilled by the next `Indexer::sync`.
  pub const PAYLOAD_SCHEMA: u64 = 2;
  ```
- `Existing` (`:444-450`) gains `Rebuild` ("its embedder is this one's, and its payload stamp is missing
  or another version: drop it, create it again, let the next sync refill it").
- `check_existing` (`:456-491`) gains `payload: Option<serde_json::Value>` after `stored`; the
  matching-embedder arm returns `Matches` iff `payload == Some(json!(PAYLOAD_SCHEMA))`, else `Rebuild`.
  The no-embedder arm stays `Stamp` (never drops, PD2). `existing()` (`:845-866`) reads
  `c.metadata.get(PAYLOAD_SCHEMA_KEY)`. `metadata()` (`:880-885`) returns both keys, so Create and Stamp
  write both.
- `check_collection` (`:810-842`):
  ```rust
  Existing::Rebuild => {
      tracing::info!(collection = %self.collection, "payload schema changed: rebuilding the collection");
      self.drop_collection().await?;
      if let Err(error) = self.create_collection().await {
          // Another process dropped and recreated it first: accept theirs if it is current now.
          if self.client.collection_exists(&self.collection).await.unwrap_or(false)
              && self.existing().await? == Existing::Matches
          {
              return Ok(());
          }
          return Err(error);
      }
  }
  ```
  `ensure_collection`'s index loop (`:793-806`) recreates the payload indexes (runs every time).
- `ExistingCollection::open` (`:974-983`): "`Matches`, `Stamp` and `Rebuild` all pass: only
  `check_collection` writes" — the read-only open never drops.
- Docs: `vector.rs:35-38` "Not versioned by name (MOD-159): a payload change bumps [`PAYLOAD_SCHEMA`],
  and `connect` drops and recreates a collection stamped otherwise; the next sync refills it. `v2` is
  a historical suffix."; `connect` doc `:707-711` adds the drop; `drop_collection` doc `:770` "tests,
  and `check_collection`'s rebuild" (already `pub`).

### T2.6 Tests in `crates/htui-store/tests/**` (write first)

**`migrations.rs`** — staged like `migration_0019_scopes_only_one_workspace_graph_runs` (`:1667`):
`bare_db()`, `MIGRATOR.run_to(22)`, plant, `MIGRATOR.run_to(23)`. `insert_run` (`:1583`) plants no
`item_id`, so add helpers `plant_item(pool, (project, kind), number, status, closed_at: Option<&str>,
user) -> Uuid` (raw INSERT, `updated_at = PLANTED_AT`) and `insert_item_run(pool, project, item, kind,
status, queued_at: &str, box_id, user) -> Uuid` (graph runs get `'{}'::jsonb`; terminal ones
`finished_at`). Runtime `sqlx::query` only (no new `.sqlx`).

1. `migration_0023_maps_every_removed_status` (all planted with `updated_at = PLANTED_AT`):

   | Key | Planted | Expect |
   |---|---|---|
   | RES-1 | `queued` + queued graph run | `open`, no note |
   | RES-2 | `in_progress` + running run | `open` |
   | RES-3 | `awaiting_approval` + awaiting_approval run | `open` |
   | RES-4 | `in_progress` + **stopped** run | `open` (Δ2) |
   | RES-5 | `queued`, no run | `open` |
   | RES-6 | `failed` + failed run | `open`, no note |
   | RES-7 | `failed`, no run | `blocked` + note "failed outside a run before the status migration; clear to run it again" |
   | RES-8 | `failed`, only a cancelled graph run | `blocked` + note |
   | RES-9 | `done` + done run, `closed_at` set | `open`, `closed_at` NULL, no note |
   | RES-10 | `done`, no run | `closed`, resolution `done`, `closed_at` NOT NULL, note "closed by the status migration: marked done with no finished run" |
   | RES-11 | `done`, latest graph run failed | `closed`/`done` + note |
   | RES-12 | `done`, done run then a later cancelled one | `open`, no note |
   | RES-13 | `done`, only a **chat** run `done` | `closed`/`done` + note (kind filter) |
   | RES-14 | `open`, `blocked`, `closed`/`withdrawn` | untouched |

   Assert per row status, resolution, `closed_at IS NULL` for every non-closed row, `updated_at =
   PLANTED_AT` for **every** row; exactly four notes, bodies verbatim, `created_by = item.created_by`;
   trigger enabled again (`pg_trigger.tgenabled = 'O'`, as `:3524`); `item_status_check` gone,
   `chk_item_status` present and `convalidated`.
2. `migration_0023_refuses_two_live_graph_runs_naming_the_items` — at v22: RES-1 `in_progress` with
   running + stopped graph runs; RES-2 with queued + awaiting_approval; RES-3 one live graph + one live
   chat run. `MIGRATOR.run_to(23, &db.pool).await.expect_err(..)`; `err.to_string()` contains `RES-1`
   and `RES-2`, not `RES-3`; `_sqlx_migrations` still 22 rows; RES-1 still `in_progress`; no
   `uq_run_item_live` in `pg_indexes`. (Scratch DB: `bare_db()`.)
3. `migration_0023_backs_one_live_graph_run_per_item` (`fresh_db`): a second live graph run refused with
   23505 / `uq_run_item_live` for each pair of live statuses incl. `stopped`; a live chat run beside a
   live graph run accepted; a graph run beside `done`/`failed`/`cancelled` accepted; an item insert
   with `queued` refused 23514 / `chk_item_status`.
4. `uq_run_item_live_matches_run_status_is_active` — `pg_get_indexdef('uq_run_item_live'::regclass)`
   names exactly `RunStatus::ALL.filter(is_active)`.

**Pins (same commit as the migration file, PD3):** `migrations.rs:118-129` applied list gains `23`
("…and MOD-144's 0023_item_status.sql"); `:3561-3562`, `:3662-3663`, `:3671-3672`, `:3694-3695`
`Pending(22)`/`MigrationsPending(22)` → 23 ("twenty-three … through MOD-144's 0023_item_status.sql");
`:3845-3848` `_sqlx_migrations` count 22 → 23; `tests/connect.rs:140-142, 156-158, 242-244` → 23.
`TABLES` (51) and `read_only_pg.rs:166` (derived) do not move. `pg_conformance.rs:43` `EXPECTED_CASES`
206 → **215** (doc chain and message `:49-56`: "MOD-144's nine item-status cases").

**`pg_criteria.rs`**

| Test | Anchor | Change |
|---|---|---|
| `status_cas_never_bumps_version` | `:550-631` | `open → blocked → open`; `open → closed` is `Constraint`, no write; version and revisions unchanged; `closed_at` `None`; stale `blocked → open` from `open` is `Ok(false)`; NotFound outranks illegal `open → closed` |
| `finish_run_holds_the_item_while_another_run_is_live` | `:5026-5145` | **Delete** (PD10). Add `two_creates_racing_on_one_item_admit_one` (two connections behind a held `SELECT … FOR UPDATE` on the item, the `claim_racing_a_pause` holder shape: one `Ok`, one `Constraint` with `startable`'s sentence, one live run) and `finishing_a_run_never_writes_its_item` (done/failed/cancelled leave `status`, `updated_at`, `closed_at` byte-identical, raw read) |
| `a_missing_tags_run_aimed_at_another_box_is_not_claimable` | `:5237` | item `Status::Open` |
| `a_step_document_racing_a_park_never_deadlocks` | `:7140-7166` | expect `Some(Status::Open)`; delete the item `transition` `:7160-7166`; doc drops "in-progress item" |
| `a_claim_racing_a_pause_admits_first_or_finds_the_run_cancelled` | `:9380-9398` | `claim_racing_a_pause` (`:9403`) returns the latest graph run status; expectations `Cancelled` / `Running` |
| `prune_finished_entries_locks_the_entries_in_item_id_order` | `:11135` | plant `SET status = 'closed', resolution = 'done', closed_at = now()` |
| `concurrent_moves_and_prunes_never_deadlock` | `:11179, 11190` | same plant; reopen `SET status = 'open', resolution = NULL, closed_at = NULL` |
| `queue_rows_answer_alike_on_both_stores` | `:10900+` | add `ana.latest_run_status == Some(RunStatus::Queued)`; `queue_facts_mem` doc `:10690-10691` |

New Pg-vs-Mem parity (PD5): `queue_rows_keep_a_cancelled_latest_run_beside_the_latest_status`,
`ready_items_skip_cancelled_runs_and_wait_on_a_rework_run`, `prune_drops_finished_and_closed_entries_alike`.

**`cache.rs`**: `the_mirror_lists_waiting_candidates_like_postgres` (`:640-940`): drop the item transition
`:782-787`; `:829-834` `InProgress → Blocked` → `Open → Blocked`; `:838-843` `Queued → Blocked` → `Open →
Blocked`; re-derive the expected ids `:885-895` from T1's fixture. `mirror_reads_equal_the_reference_store`
(`:323`): one `ItemFilter { states: Some(vec![s]), .. }` per `ItemState`, one with two values, one
`ready: Some(false)` (`ItemSummary` equality now covers `latest_run_status` — the cache-vs-Mem drift
guard). New `the_mirror_derives_item_state_like_postgres`: `items(ready)`, `items(states)`, `links`,
`upstream_summaries`, `requirement_coverage` equal to `PgStore` over the parity rows.

**Search**: `vector.rs` units beside `a_collection_without_an_identity_is_stamped` (`:1566`):
`a_current_payload_stamp_matches`, `an_older_payload_stamp_is_rebuilt`,
`a_missing_payload_stamp_on_a_stamped_collection_is_rebuilt`,
`a_collection_without_an_identity_is_stamped_not_rebuilt`, `the_metadata_records_the_payload_schema`
(existing `check_existing` calls gain the argument). `tests/qdrant_live.rs` (serial, skip without
`HTUI_TEST_QDRANT_URL`): `a_new_collection_records_the_payload_schema`,
`an_old_payload_schema_drops_and_recreates_the_collection` (connect, upsert one point, metadata
`{payload_schema: 1}` via `raw()`, reconnect: zero points, stamp 2, embedder kept),
`open_existing_never_drops_an_old_payload_schema`. `an_unrecorded_collection_is_stamped_once` and
`stamping_keeps_the_collections_other_metadata` stay green unchanged. `vector_sync.rs:420
a_status_move_reaches_the_document_points` needs a fixture item that is `open` or `blocked`.

### T2.7 `.sqlx`

Entries removed / added / changed (old file deleted, new written):
- `write.rs`: `finish_run_on` run SELECT (changed), live `count(*)` and mirror UPDATE (removed; the item
  SELECT stays, shared with `close_out`); `transition` UPDATE (changed); `create_run` live SELECT (new),
  item UPDATE (removed); `claim_run` MissingTags UPDATE (changed), admitted UPDATE (removed);
  `promote_step`, `park_step` SELECTs (changed), their shared item UPDATE (removed); `close_out` latest
  SELECT (new); `prune_finished_entries` (changed).
- `read.rs`: `items`, `ready_items`, `queue_rows`, `waiting_candidates` (items statement),
  `requirement_coverage`, `links` (node statement), `upstream_summaries` (changed).
- A byte-identical statement keeps its hash (memory: sqlx offline hash = literal query).

Recipe (`docs/hr-sandbox.md:208-221`; prepare needs a DB migrated **by sqlx**, the compose `htui` DB is
empty and is not the test DSN):
```bash
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;"   # 0023 edited since -> checksum mismatch
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features   # both flags, or test/demo queries drop out
cargo sqlx prepare --check -- --all-targets --all-features
git status --short .sqlx                             # D for removed statements, A for new
```
While iterating: `SQLX_OFFLINE=false DATABASE_URL=… cargo check -p htui-store --all-targets --all-features`
(a process env var outranks the non-forced `[env]` `SQLX_OFFLINE=true`, `.cargo/config.toml:2-6`).
"potentially unused queries" is expected.

### T2.8 Commit groups
1. `test(MOD-144): 0023 item status migration and its tests` — `migrations/0023_item_status.sql`,
   `migrations.rs` tests 1-4 + pins, `connect.rs` pins.
2. `feat(MOD-144): Pg writes stop mirroring runs onto the item` — `write.rs`.
3. `feat(MOD-144): Pg reads derive the latest graph run; .sqlx` — `read.rs`, `rows.rs`,
   `item_state_case_matches_item_state_of`, `.sqlx/**`. The crate builds from here (C2+C3 may be one
   commit if `cargo check` must stay green).
4. `feat(MOD-144): cache derives item state like Postgres` — `cache/read.rs`, `cache.rs`.
5. `feat(MOD-144): search payload-schema stamp (Decision B)` — `vector.rs`, `qdrant_live.rs`.
6. `test(MOD-144): Pg criteria and conformance pin follow the three statuses` — `pg_criteria.rs`, `pg_conformance.rs:43`.

### T2.9 Verification
```bash
cargo check  -p htui-store --all-targets --all-features             # offline, after prepare
cargo test   -p htui-store --all-features -- --test-threads=1        # HTUI_TEST_DATABASE_URL, HTUI_TEST_QDRANT_URL
cargo test   -p htui-store --all-features --test migrations -- --test-threads=1 migration_0023
cargo test   -p htui-store --all-features --test pg_conformance -- --test-threads=1
cargo clippy -p htui-store --all-targets --all-features -- -D warnings
cargo clippy -p htui-store -- -D warnings
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
```
`pg_conformance` SIGABRT: check the stack note before calling it a regression; `qdrant_live` timeouts:
re-run serially. Leftover sweep with Gortex `search_text` over `crates/htui-store/src` (regexp
`'in_progress'|NOT IN \('done','closed'\)|finish_run_item_mirror|Status::(Queued|InProgress|AwaitingApproval|Done|Failed)\b`):
zero hits except `run.status`/`run_step.status` literals.

---

## T3 — htui-orch (after T1; parallel with T2)

### T3.1 Test helper (`src/fake.rs`, `test-support`)
```rust
/// MOD-144: the item's derived display state: its stored status over its latest graph run.
pub async fn state_in(store: &MemStore, id: ItemId) -> ItemState {
    let item = store.item(id).await.expect("MemStore never fails a read").expect("the item exists");
    let runs = store.runs(id).await.expect("MemStore never fails a read");
    ItemState::of(item.status, latest_graph_run(&runs).map(|run| run.status))
}
impl FakeOrchestrator { pub async fn state(&self, id: ItemId) -> ItemState { state_in(self.store(), id).await } }
```
beside `FakeOrchestrator::item` (`fake.rs:2477`); `conformance.rs` gets `async fn state_of<O:
Orchestrate>(orch: &O, id: ItemId) -> ItemState` next to `item_of` (`:1135`).

### T3.2 `src/command.rs`
`use` (`:29`): add `ItemState`, `RunKind`, `latest_graph_run`, `GraphSnapshot`.
```rust
/// §6.2's `run`/`queue` over the item row: the mirror of `create_run`'s own check (MOD-144:
/// `open` and no live graph run, `htui_core::store::startable`), so the Runs tab greys `R` by the
/// same law. **The engine does not call it** for the decision: `create_run` refuses under the item
/// lock. `start_enabled_is_the_law` pins the two together.
///
/// # Errors
/// [`EngineError::Store`] carrying `startable`'s own refusal.
pub fn start_enabled(item: &Item, runs: &[Run]) -> Result<(), EngineError> {
    let live = runs.iter()
        .filter(|run| run.kind == RunKind::Graph && run.status.is_active())
        .max_by_key(|run| (run.queued_at, run.id))
        .map(|run| run.status);
    htui_core::store::startable(item.status, live).map_err(EngineError::Store)
}

/// §4.3 verdict 1's `unblock` (MOD-4 plan D161, MOD-144). `runs` is every run of the item, each
/// paired with whether it is resumable (`false` for a finished run). Only live runs decide the
/// case; the latest graph run names the item's state in a refusal.
///
/// 1. `blocked` and no run is live: [`UnblockCase::Reopen`].
/// 2. `blocked` and a live run is parked: [`UnblockCase::FollowRun`].
/// 3. `open` over a parked run that is resumable: [`UnblockCase::Resume`].
/// An `open` item over a parked run that is not resumable waits at a gate; one over a `queued`,
/// `running` or `stopped` run is walking (a stopped run resumes through MOD-125's resume).
pub fn unblock_enabled(item: &Item, runs: &[(Run, bool)]) -> Result<UnblockCase, EngineError> {
    let active: Vec<&(Run, bool)> = runs.iter().filter(|(run, _)| run.status.is_active()).collect();
    let parked = active.iter().find(|(run, _)| run.status == RunStatus::AwaitingApproval);
    let why = match (item.status, parked, active.first()) {
        (Status::Blocked, _, None) => return Ok(UnblockCase::Reopen),
        (Status::Blocked, Some((run, _)), _) => return Ok(UnblockCase::FollowRun(run.id)),
        (Status::Open, Some((run, true)), _) => return Ok(UnblockCase::Resume(run.id)),
        (Status::Open, Some((run, _)), _) => run_waits_at_a_gate(run.id),
        (Status::Blocked | Status::Open, None, Some((run, _))) => run_is_walking(run.id, run.status),
        _ => nothing_is_blocked(),
    };
    let latest = latest_graph_run(runs.iter().map(|(run, _)| run)).map(|run| run.status);
    Err(EngineError::NotBlocked { item: item.id, state: ItemState::of(item.status, latest), why })
}

/// `R-TUI-9`'s close-out (MOD-4 plan D167, ANA-35 §7.1's law): no run of the item is live, the item
/// is not `closed`, and the picker starts on `Resolution::default_for` the latest graph run:
/// `concluded` when it finished and its snapshot's last phase outputs `verdict`, `done` when it
/// finished otherwise, `withdrawn` when nothing finished.
///
/// # Errors
/// [`EngineError::RunStatus`] naming the first live run, then [`EngineError::NotClosable`] for a
/// `closed` item.
pub fn close_out_enabled(item: &Item, runs: &[Run]) -> Result<Resolution, EngineError> {
    if let Some(live) = runs.iter().find(|run| run.status.is_active()) {
        return Err(EngineError::RunStatus { run: live.id, status: live.status, expected: "done | failed | cancelled" });
    }
    if item.status == Status::Closed {
        return Err(EngineError::NotClosable { item: item.id, state: ItemState::Closed });
    }
    let finished = latest_graph_run(runs).filter(|run| run.status == RunStatus::Done);
    let kind = finished.map(|run| run.graph_snapshot.as_ref()
        .and_then(GraphSnapshot::last_output_kind).unwrap_or_default());
    Ok(Resolution::default_for(kind.as_deref()))
}
```
`EngineError` (`:609-668`), field `status` → `state`:
```rust
#[error("item {item} is `{}`; {why}", .state.word())]
NotBlocked { item: ItemId, /// Its display state (MOD-144). state: ItemState, why: String },
/// Plan D167, ANA-35 §7.1: close-out needs an item that is not `closed`.
#[error("item {item} is `{}`; close-out needs an `open` or `blocked` item (ANA-11 §4.2)", .state.word())]
NotClosable { item: ItemId, state: ItemState },
```
Constructors/patterns: `command.rs:1239`, `:1265`, `engine.rs:1747` (Reopen stale branch: `state:
ItemState::of(row.status, None)`), tests `command.rs:1864-1882, 1933, 2257, 2340`, `engine.rs:15306`,
`conformance.rs:6373`. No htui/htui-mcp code matches these fields.
Call sites of the predicates: `start_enabled` — only `htui-worker/src/views.rs:335` (T3b);
`unblock_enabled` — `engine.rs:2509-2523` (`unblock_case`), `views.rs:470-480`; `close_out_enabled` —
`views.rs` `verdicts`. `retry_admitted(.., item: Status, ..)` (`:1067`) and `promote_enabled(.., item:
Status, ..)` (`:1122`) keep their signatures (they test `== Status::Blocked`).
Docs that become false: `:15` module doc ("`create_run`'s live-run check"); `:325-331` `UnblockCase`
(Reopen "no run is live"; FollowRun "the item goes back to `open` (`blocked → open`); the parked run's
`awaiting_approval` carries *needs you* (R-4)"; Resume "the item is `open` over this run, parked where
a crashed command or refused reconcile left it"); `:398-401` `ItemBlocked` (drop the `can_move_to`
anchor; "a `blocked` item is not walked until `Unblock` clears it"); `:655-657`; `:1204-1218`;
`:1243-1255`; `:1283-1295`.

### T3.3 `src/closeout.rs`
`Preview` (`:18-34`) gains, after `status`, `/// Whether the item's latest graph run finished `done`
(MOD-144): the second input of `Resolution::closes_from`. pub finished: bool,`; `status` doc "`open` or
`blocked`"; `:26-27` "`closes_from(status, finished)`". `pub fn preview(item: &Item, resolution:
Resolution, finished: bool, runs: &[RunSummary], commits, heads) -> Preview`. Only caller
`engine.rs:2534`: `let finished = latest_graph_run(&reads.runs).is_some_and(|run| run.status ==
RunStatus::Done);`. Tests `:239` → `Status::Open`; `:605` literal `status: Status::Open, finished: true`.

### T3.4 `src/engine.rs` per call site

**Drop the 7 mirror calls** (and the `if let Some(item)` wrapper when it holds nothing else):

| Fn (anchor) | Drop | Keep | Doc that becomes false |
|---|---|---|---|
| `cancel_queued` (`:1884-1912`) | `:1895-1901` (whole `if let Some(item) = row.item_id { … transition(item, Queued, Open) … }`) | run CAS, `cleanup_run`, outcome | `:1876-1883` incl. the **E-2 (accepted)** paragraph → "`queued → cancelled` as a compare-and-set, then plan D36's cleanup. The item is not written (MOD-144): its state is derived from its runs, and the cancelled run is skipped." |
| `cancel_leased` comment (`:1932-1934`) | — | `finish_run` | "`finish_run` mirrors the item … which is what frees it" → "`finish_run` ends the live run, which frees the item for a second run" |
| `recover_run` (`:2632-2641`) | `:2636-2639` | `move_run`, re-read, `Next::Parked` | `:2632` "`gate::park`'s run and item writes" → "run write"; `item` (`:2600`) still used by `recover_step` |
| `park_interrupted` (`:2979-3005`) | `:2990-2994` | the `if let Some(item)` with the `note` | `:2976-2978` → "the run, then the note" |
| `resume_window` topology branch (`:3227-3238`) | `:3230-3234` | `move_run`, re-read | `:3224-3226` "— run, then item —" → "— the run —" |
| `park_selection` (`:4801-4830`) | `:4812-4816` | note body | `:4793-4795` delete "the item `in_progress -> awaiting_approval`," |
| `park_run` (`:5636-5667`) | `:5640-5644` | `add_note` | `:5627-5631` "run, item, then the note" → "run, then the note"; delete the "item goes through `in_progress`" sentence |
| `unpark` (`:6807-6820`) | `:6813-6818` | `transition_run` CAS, `Ok(unparked)` | `:6804-6806` → "The run back to `running`" |

Other false comments: `answer_guarded` `:961-965` → "the escalation's `move_run(Running →
AwaitingApproval)` (gate.rs `escalate`) needs the run `running`, so the unpark comes first (blueprint
A-5)"; `admit` `:3316-3319` → "blocking first keeps the refusal readable if a crash cuts the tail off
(an open item over a failed run reads `failed`, not `blocked`)"; `close_out` `:1797-1807` → "an open
or blocked item; `default_for` reads the latest graph run"; test-harness `:7915-7921`.

**Blocking writes** (`from` is `Open`):

| Site | Old → New | Notes |
|---|---|---|
| `refuse_rung_four` `:867` | `Open → Blocked` unchanged | doc `:855-859`: "only an item `startable` admitted reaches here (PD1), so the move lands; `Ok(false)` means another writer moved it since; the note is still written (invariant 7)" |
| `refuse_missing_tags` `:890` | unchanged | doc `:880-884` same |
| `refuse_no_candidate` `:3484` | `InProgress → Blocked` → `Open → Blocked` | `Ok(false)` ignored as today; comment `:3487-3488` → "`finish_run` no longer writes the item (MOD-144). The order (item, note, run) stays so a crash leaves the refusal readable first"; doc `:3450-3451` |
| `block_and_fail` `:5785` | `InProgress → Blocked` → `Open → Blocked` | doc `:5771-5774` → "the item `open -> blocked`, the failure as its note, then `finish_run(Failed)` (which writes no item) and cleanup" |
| `gate.rs::escalate` `:1063` | `ctx.move_item(InProgress, Blocked)` → `ctx.move_item(Status::Open, Status::Blocked)` | `move_item` keeps D17's `Ok(false)` |

gate.rs docs: `:341-343` (drop the `item.rs:46-60` anchor), `:380-383` ("`park_step` writes no item
(MOD-144)"), `:389-390` (drop the mirror clause), `:713` ("the run to `running` if it was parked").

**`unblock`** (`:1723-1786`): FollowRun `:1754-1770` → `transition(item, Status::Blocked, Status::Open)`,
`StaleWrite { to: Status::Open.to_string(), .. }` (`:1767`), comment `:1755-1757` "the item goes back to
`open` and its parked run's `awaiting_approval` is what reads *needs you*"; the note text stays. Reopen
`:1735-1751` unchanged CAS, stale branch `NotBlocked { state: ItemState::of(row.status, None), why:
nothing_is_blocked() }`. Doc `:1724-1731`.

**`unblock_case`** (`:2509-2523`), PD9:
```rust
let row = self.item(item).await?;
let mut runs = Vec::new();
for summary in self.parts.store.runs(item).await? {
    let run = self.run(summary.id).await?;
    let resumable = if run.status.is_active() {
        let snapshot = Self::snapshot_of(&run)?;
        let steps = self.parts.store.run_steps(run.id).await?;
        resumable(&cursor(&snapshot, &steps), &steps)
    } else { false };
    runs.push((run, resumable));
}
crate::command::unblock_enabled(&row, &runs)
```
**`close_out_preview`** (`:2532-2543`): compute `finished`, pass it to `closeout::preview`.

**`enqueue_with`** (`:710-772`), PD1 — replace `:720-733`:
```rust
let item = self.item(item).await?;
// MOD-144: `create_run`'s own law first (`startable`: `open` and no live graph run), so neither
// the tag check nor rung 4 writes a note on, or blocks, an item some live run already holds;
// `create_run` re-checks it under the item lock and stays the authority (MOD-7 D98).
let live = self.parts.store.runs(item.id).await?.into_iter()
    .filter(|run| run.kind == RunKind::Graph && run.status.is_active())
    .max_by_key(|run| (run.queued_at, run.id))
    .map(|run| run.status);
htui_core::store::startable(item.status, live)?;
let missing = self.parts.graphs.missing_tags(item.id, self.parts.box_id).await?;
if !missing.is_empty() { return Err(self.refuse_missing_tags(&item, missing).await?); }
```
Comment `:753-756` → "`create_run` refuses an item that is not `open` or has a live graph run, under
its lock; the check above is its mirror". Behaviour change: StartRun on a `blocked` item no longer
reaches rung 4 (it gets `startable`'s Constraint with no note — D98's "writes no note").

### T3.5 Tests first

| Test (path:line) | Change |
|---|---|
| `start_enabled_is_the_law` `command.rs:2370` | every `Status::ALL` × `live ∈ {None} ∪ live RunStatus`: `start_enabled(&item, &runs).is_ok() == (status == Open && live.is_none())`, equals `startable(..)`, refusal `Store(Constraint(s))` with `s == illegal_move("item", ItemState::of(status, live), "queued")`; `open` over `done`/`failed`/`cancelled` admitted; a live chat run does not refuse |
| new `close_out_defaults_from_the_latest_graph_run` | `(Open, [])`→Withdrawn; `(Open,[failed])`→Withdrawn; `(Open,[done])`→Done; `(Open,[done, newer cancelled])`→Done; `(Open,[done, newer failed])`→Withdrawn; `(Blocked,[done])`→Done; each `closes_from(status, finished)` |
| new `close_out_concludes_after_a_verdict_phase` | snapshot last phase `verdict`, `done` → `Concluded`; `graph_snapshot: None` → `Done` |
| `close_out_needs_no_live_run_and_a_closable_item` `:2301` | keep the live-run refusal; `item_at(Closed)` → `NotClosable { state: Closed }`; delete the old `default_for(status)` assertion (`:2312`) |
| `unblock_names_its_three_cases_and_what_holds_the_item` `:2201` | `item_at(Open)` Resume / gate refusal "item {} is \`needs you\`; run {} is parked at a gate; answer it"; Blocked+running "…\`blocked\`; run {} is \`running\`"; new rows Open+running, Open+stopped (`run_is_walking`); `[Open, Closed]` with no runs → `nothing_is_blocked`; Open+`[done]` → state `Done` |
| `unblock_resumes_a_run_parked_over_a_crashed_rejection` `:2269` | `item_at(Open)` |
| `the_milestone_6_refusals_say_what_went_wrong` `:1859` | NotBlocked rows `state: ItemState::{Open, Blocked, NeedsYou}`; NotClosable `Closed`, "item {item} is \`closed\`; close-out needs an \`open\` or \`blocked\` item (ANA-11 §4.2)" |
| `retry_admitted_is_retry_step_s_order` `:1987, 2017, 2022, 2035, 2043`; `promote_is_enabled_on_a_live_step_of_a_live_run` `:2062, 2075, 2092, 2125, 2137` | `Status::AwaitingApproval` → `Status::Open` |
| engine `a_feature_graph_walks_its_four_phases` `:9004, 9030, 9033` | `state(FEAT_3) == NeedsYou` then `Done`; `item.status == Open`; `:9033` `closed_at.is_none()` (Δ10) |
| engine `:9155`, `:9893-9894`, `:11634` | `state == Failed` |
| engine `failed_under_a_running_run` `:11583-11597` | delete the item `transition(AwaitingApproval → InProgress)` assert; doc `:11514-11516` |
| engine `:11711`, `:13477`, `:14268`, `:14406` | `state == NeedsYou`; `:14406` also `item.status == Open` (FollowRun); `:14297` state Failed; doc `:14243` |
| engine `claim_refused_names_the_rule` `:12215`, `enqueue_then_claim_is_start_run` `:14588`, `:21906` | `state == Queued`, item `Open` |
| engine `enqueue_refuses_missing_tags_on_a_failed_item_and_leaves_it_failed` `:14715` | rename `…_and_blocks_it` (PD13): `finish_run(RUN_2, Failed)` (drop the `:14725` transition); precondition state `Failed`; after enqueue `MissingTags`, item `Blocked`, note `missing tags: vulkan`, no run; delete `:14781-14789` |
| engine `enqueue_checks_tags_only_where_create_run_would` `:14820` | `:14842` `state == Queued`; error `EngineError::Store(_)` from `startable`; no note |
| engine `unblock_case_reads_the_item_and_its_active_runs` `:15291` | rename `…_and_its_runs`; `NotBlocked { state: ItemState::Open, .. }` |
| engine `close_out_preview_counts_what_close_out_writes` `:15329` | `(preview.status, preview.finished, preview.resolution, runs, rows, version) == (Status::Open, true, Resolution::Done, 2, 4, 1)` |
| engine `:22028` | `state == Running` |
| engine `a_queued_cancel_moves_the_item_back_to_open` `:22032` | rename `a_queued_cancel_frees_the_item_and_writes_no_item_row`: `version`/`updated_at` unchanged, `state != Queued`, a second StartRun admitted |

**Conformance (`src/conformance.rs`)**: every removed-variant assertion on `item_of(..).status` →
`state_of(&orch, id).await == ItemState::…` (stored `Status::Open` only where the point is the stored value).

| Case (line) | New assertion |
|---|---|
| helper `free_feat_3` doc `:1173-1181` | "FEAT-3 is seeded `open` with a live `queued` RUN_2, so `create_run` refuses StartRun until RUN_2 ends; cancelling writes no item row" |
| helper `approve_without_walking` `:1462-1487` | drop the third write (`:1480-1483`); doc "two writes" |
| `feat_walks_end_to_end` `:1557-1560, 1578-1586` | `NeedsYou`; `Done`; `closed_at.is_none()` (Δ10); doc `:1521` |
| `:1717`, `:2564`, `:2833`, `:3915`, `:6465`, `:7915` | `ItemState::Done` |
| `:1774`, `:1873`, `:2725`, `:3837`, `:5111`, `:5433`, `:5650`, `:5703`, `:5788`, `:6018`, `:6491`, `:6628`, `:7025`, `:7062`, `:7296` | `ItemState::NeedsYou` |
| `:2292`, `:2333`, `:7109` | `ItemState::Failed` |
| `finish_run_is_the_last_write` `:2434-2477` | name kept; prologue state `Queued`, item `Open`; after `free_feat_3` state `Open`, item `version`/`updated_at` unchanged; tail `Done`, `closed_at.is_none()`; rewrite doc `:2433-2441`, delete `:2472-2476` |
| `overlapping_touched_paths_serialise` `:4441` | `ItemState::Queued` |
| `a_third_run_waits_for_a_slot_…` `:4554-4563` | drop the item `transition` assert; doc `:4516-4518` |
| `unblock_opens_a_blocked_item_with_no_run` `:6370-6376` | `NotBlocked { state: ItemState::Open, .. }` |
| `unblock_lets_an_escalated_run_be_promoted_and_approved` `:6440-6445` | `item.status == Open`, state `NeedsYou`; doc `:6389-6392` |
| `fanout_paths::unpark_by_hand` `:8475-8495` | drop the item `transition` assert and doc clause |
| module doc `:11` | "the item's derived state" |

New cases (append to `CASES` and the `run_case` match): `a_queued_cancel_writes_no_item` (item row equal
before/after, state `Open`, StartRun admitted after); `a_stopped_run_holds_its_item` (run stopped by
hand: state `Stopped`; StartRun → `Store(Constraint(illegal_move("item", ItemState::Stopped,
"queued")))`; Unblock → `NotBlocked { state: Stopped, why: run_is_walking(run, Stopped) }`; CloseOut →
`RunStatus { status: Stopped }`); `a_rework_run_on_a_finished_item_is_admitted` (approve to `Done`;
StartRun admitted → `NeedsYou`; CancelRun → `Done`; `CloseOut { Done }` succeeds).
Pins: `conformance.rs:8391` and `tests/fake_conformance.rs:18` **109 → 112** together (message: "and
MOD-144's three"). `tests/{fake_conformance,fixtures,gix_isolator,infer,review_loop}.rs` hold no
removed-variant reference.

### T3.6 Commit groups
1. `refactor(MOD-144): orch predicates and close-out preview over derived item state` — command.rs,
   closeout.rs, engine.rs, gate.rs production + their unit tests written first. `cargo check -p htui-orch`.
2. `test(MOD-144): orch unit tests read the item's derived state` — fake.rs helpers, test modules.
   `cargo test -p htui-orch --all-features --lib`.
3. `test(MOD-144): orch conformance over ItemState; three new cases` — conformance.rs, fake_conformance.rs pin.

### T3.7 Verification
```bash
cargo check -p htui-orch
cargo test -p htui-orch --all-features
cargo clippy -p htui-orch --all-features --all-targets -- -D warnings
cargo clippy -p htui-orch -- -D warnings
```
Engine walks sit near the 2 MiB debug stack: look for SIGABRT/stack overflow before reading asserts.

---

## T3b — htui-worker (after T2 and T3; parallel with T4)

- `views.rs::verdicts` (`:323-421`): `let rows: Vec<Run> = runs.iter().map(|(run, _)| run.clone()).collect();`
  then `run: start_enabled(item, &rows).map_err(sentence)`, `close_out: close_out_enabled(item,
  &rows).map(|_| ()).map_err(sentence)`. `retry_admitted` (`:404`) / `promote_enabled` (`:406-408`) unchanged.
- `views.rs::unblock_case` (`:470-480`), PD9:
  ```rust
  fn unblock_case(item: &Item, runs: &[(Run, Vec<RunStep>)]) -> Result<UnblockCase, String> {
      let mut all = Vec::with_capacity(runs.len());
      for (run, steps) in runs {
          let resumable = if run.status.is_active() {
              let snapshot = snapshot_of(run).map_err(|err| err.to_string())?;
              resumable(&cursor(&snapshot, steps), steps)
          } else { false };
          all.push((run.clone(), resumable));
      }
      unblock_enabled(item, &all).map_err(|err| err.to_string())
  }
  ```
  Doc `:466-469`: "`runs` is every run of the item in `ReadStore::runs` order; the waiting list passes
  a candidate's live runs only and discards the refusal". `REOPEN_TEXT`/`FOLLOW_TEXT`/`RESUME_TEXT`
  (`:577-581`) stay true.
- No `LiveFacts`/`classify_entry` in htui-worker. `runtime.rs` admit (`:2175-2330`) reads
  `ready_items`/`batch_cancelled_items`/`prune_finished_entries` — semantics move in T1/T2, no code
  change. Docs: `runtime.rs:2152` "whose item is closed or finished (latest graph run `done`)";
  `:2225-2227` "A cancel ends the run with the entry kept (D9); the item reads as its last real run left it".
- Tests (`views.rs`): `:1523, 1548, 1567, 1592` `AwaitingApproval` → `Open` (doc `:1520-1521`);
  `:1615, 1935, 2060, 2066, 2107, 2318` `over_a_gate(Status::Open)` (delete the duplicate `:2066`,
  `:2069`); `:2041, 2118, 2158, 2178, 2199, 2205, 2215` → `Status::Open`; `:1779` stays (now
  `run_waits_at_a_gate`); `:1175` unchanged; `:1283` → `Status::Open` + `state(FEAT_3) == NeedsYou`;
  new `start_is_refused_over_a_live_run_and_admitted_over_a_finished_one`.
- `tests/auto_queue.rs`: `a_parked_auto_run_keeps_a_stalled_batch_open` `:1762, 1773` → `Status::Open`
  ("its run parks it (*needs you*), so not ready"); docs `:1312`, `:1387`, `:1761-1762`.
- Commit: `refactor(MOD-144): worker verdicts over derived item state`.
- Verify (needs T2's `.sqlx` and a DB migrated to 23):
  ```bash
  cargo test -p htui-worker --all-features -- --test-threads=1
  cargo clippy -p htui-worker --all-features --all-targets -- -D warnings
  cargo clippy -p htui-worker -- -D warnings
  ```

---

## T4 — htui-mcp (after T2 and T3)

- `src/tools/status.rs`: `status_schema` (`:33-36`) maps `Status::ALL` (shrinks by itself);
  `DEF.description` (`:52-53`) → `"Asks a human to set this step's item to open, blocked or closed;
  writes a note, never the status itself."`; `:69`, `:79` unchanged. Keep the comment-not-doc
  convention on `StatusArgs` fields (`:14-17`): schemars turns `///` into agent-visible text.
- `src/tools/search.rs` (`:54-60`): automatic; `SearchArgs.statuses` doc (`:35`) stays.
- Tests (`tests/tools_backlog.rs`): `:464` → `{"status": "blocked", "reason": "every test passes"}`,
  note `"status request: blocked \u{2014} every test passes"`; `:527` stray-resolution call with
  `"open"`, plus `{"status": "done"}` refused `starts_with("invalid arguments: unknown variant `done`")`;
  `:678` → `"blocked"`; new `item_status_offers_exactly_the_three_statuses` (`tools/list`: both enums
  `["open","blocked","closed"]`).
- Snapshots that move (only these): `src/tools/snapshots/htui_mcp__tools__box_profile__tests__tools_list_per_scope_shape.snap`,
  `tests/snapshots/tools_search__a_fresh_chat_scope_sees_box_profile_and_search_concepts.snap`.
- Commit: `feat(MOD-144): htui-mcp item_status and search_concepts offer three statuses`.
- Verify: `cargo test -p htui-mcp --all-features`; `cargo insta pending-snapshots` (diff each);
  `cargo clippy -p htui-mcp --all-features -- -D warnings`.

---

## T5 — htui TUI and binary (after T3b and T4)

### T5.1 Theme (`ui/theme.rs`)
Import `ItemState` (`:15`). Replace `:261-271`:
```rust
/// The style an [`ItemState`] renders with, so every list agrees on what "blocked" looks like.
/// `stopped` reads as plain text, as its run mark does (`kit::label`'s `Tone::Idle`).
#[must_use]
pub fn status_style(&self, state: ItemState) -> Style {
    match state {
        ItemState::Open | ItemState::Queued | ItemState::Stopped => self.base,
        ItemState::Running => self.running,
        ItemState::NeedsYou => self.warning,
        ItemState::Blocked | ItemState::Failed => self.error,
        ItemState::Done | ItemState::Closed => self.dim,
    }
}
```
Docs `:84`, `:86` → "a running item", "an item that needs you". Test `:333-347` →
`a_running_item_is_running_and_one_that_needs_you_is_a_warning`, 9 rows.

### T5.2 Kit status (`ui/kit/status.rs`)
```rust
/// Every item state with its mark, in attention order (`ItemState::ALL`, MOD-144 Decision A).
pub const TABLE: &[(ItemState, StatusMark)] = &[
    (ItemState::NeedsYou, StatusMark { glyph: "⚠", word: "needs you" }),
    (ItemState::Failed,   StatusMark { glyph: "✗", word: "failed" }),
    (ItemState::Running,  StatusMark { glyph: "◐", word: "running" }),
    (ItemState::Stopped,  StatusMark { glyph: "■", word: "stopped" }),
    (ItemState::Blocked,  StatusMark { glyph: "⊘", word: "blocked" }),
    (ItemState::Queued,   StatusMark { glyph: "◌", word: "queued" }),
    (ItemState::Open,     StatusMark { glyph: "○", word: "open" }),
    (ItemState::Done,     StatusMark { glyph: "●", word: "done" }),
    (ItemState::Closed,   StatusMark { glyph: "✓", word: "closed" }),
];
pub fn mark(state: ItemState) -> StatusMark { /* unchanged body */ }
pub fn spans(state: ItemState, theme: &Theme) -> Vec<Span<'static>> { /* theme.status_style(state) */ }
```
Tests: `:110-123` over `ItemState::ALL` (9); `:125-146` the 9 pairs (`NeedsYou` "needs you",
`Running` "running"); `:148-157` uses `ItemState::as_str`; `:159-171` unchanged (■ single-width);
`:173-184`, `:196-212` over `ItemState::ALL`; `:186-194` `Failed` vs `NeedsYou`; new
`an_item_state_reads_as_its_run_status` (Running/NeedsYou/Stopped/Queued/Done/Failed glyph+word equal
`label::run_status` of the matching `RunStatus`) and `the_word_is_item_state_word`.

### T5.3 Kit labels (`ui/kit/label.rs`)
`:81` `const STOPPED: Mark = mark("■", "stopped", Tone::Idle);` (Decision A). Docs `:3-4`, `:125` "item
states". Tests `:521-535` (rename `a_run_status_matches_the_item_state_words`, row 4 `("■", "stopped",
Tone::Idle)`) and `:538-563` row 4. `CANCELLED` `⊘` (`:84`) unchanged (PD14).

### T5.4 Leave overlay (`ui/overlay/leave.rs:139-140`)
`mark(ItemState::Running).glyph`, `mark(ItemState::NeedsYou).glyph`; import `:30`. Glyphs identical —
its 4 snapshots must not move.

### T5.5 Backlog list (`ui/tabs/backlog/list.rs`)
`lines()` (`:167-221`):
```rust
let mark_width = |item: &ItemSummary| {
    let mark = status::mark(item.state());
    cell_width(mark.glyph) + 1 + cell_width(mark.word)
};
let status_width = view.items.iter().map(mark_width).max().unwrap_or(0);   // replaces :175-180
// per item, replacing the Span at :208-211:
let mut spans = vec![Span::raw(" ".repeat(INDENT)),
    Span::styled(cells::pad(&item.key, key_width), theme.key), Span::raw(" ".repeat(GAP))];
spans.extend(status::spans(item.state(), theme));
spans.push(Span::raw(" ".repeat(status_width - mark_width(item))));
spans.push(Span::raw(" ".repeat(GAP)));
spans.push(Span::styled(cells::clip(&item.title, title_width), theme.base));
```
`groups()` (`:186`) untouched (MOD-119). Tests: `summary(..)` helper (`:272-287`) gains `latest:
Option<RunStatus>`; `:299`, `:340` FEAT-12 `(Open, Some(Running))`; `:420-447` →
`an_item_key_is_bold_and_a_running_item_is_running` (`"◐"`, fg `Color::Blue`).

### T5.6 Detail Body (`ui/tabs/backlog/detail/body.rs`), PD16
```rust
pub struct BodyTab { item: Option<Item>, /// MOD-144: the item's runs, for its display state.
    runs: Option<Vec<RunSummary>>, scroll: Scroll, drawn: Cell<u16> }
// on_item_change: also self.runs = None;   (else the previous item's runs leak)
// on_reply: StoreReply::Runs(runs) => self.runs = Some(runs.clone()),
// render (:124):
let state = ItemState::of(item.status,
    self.runs.as_deref().and_then(|runs| latest_graph_run(runs)).map(|run| run.status));
let mut key_line = vec![Span::styled(item.key.as_str(), ctx.theme.key), Span::raw("  "),
    Span::styled(item.key_prefix.as_str(), ctx.theme.dim), Span::raw("  ")];
key_line.extend(status::spans(state, ctx.theme));
```
Test `:197` `"FEAT-1  FEAT  in_progress"` → `"FEAT-1  FEAT  ◐ running"` (T1 fixture); `pane()` helper
(`:165-175`) also feeds `StoreReply::Runs(..)`.

### T5.7 Detail Graph (`ui/tabs/backlog/detail/graph.rs`)
`fixed_cells` (`:533-536`): width of `status::mark(row.node.state())` (glyph + 1 + word). `line()`
(`:556-561`): `theme.status_style(node.state())`, one string `format!("{} {}", mark.glyph, mark.word)` so
`cut()` (`:573`, `:597`) still sees one span. Test literal `node()` (`:929-938`) appends
`latest_run_status: None`. Text pins `:1248, 1478, 1479, 1496, 1500, 1583, 1622, 1812` move to T1's
fixture marks (FEAT-1 `◐ running`, TOOL-1 `⚠ needs you`); `the_narrowest_label_keeps_the_key`
(`:1580-1610`) recomputes widths.

### T5.8 Backlog filter (`ui/tabs/backlog/filter.rs`)
```rust
pub struct BacklogFilter {
    /// Item states to keep; empty = any. Kept in [`ItemState::ALL`] (attention) order.
    pub states: Vec<ItemState>,
    pub projects: Vec<ProjectId>, pub tags: Vec<String>, pub ready_here: bool,
}
// item_filter (:90-100):
ItemFilter { statuses: None, states: facet(&self.states), project_ids: facet(&self.projects),
             tags: facet(&self.tags), ready: None, text: None }
// summary (:122-124):
let names: Vec<&str> = self.states.iter().map(|s| s.word()).collect();
```
`FilterForm.status_at` indexes `ItemState::ALL` (`:206`, `:323`, `:334-338`, `:389-397` show `.word()`);
`retain_projects` doc (`:146`). Tests (`:560-1064`), `statuses` → `states`: `:579, 589`
`ItemState::ALL[1]` (Failed); `:619` `ALL.len() - 1`; `:673, 693, 712` `[Open, Done]` (`:693` expected
`ItemFilter { statuses: None, states: Some(..) }`; `:716` stays `"status:open,done · …"`); `:732, 738`
`Done`; `:790` `l` → `Failed`, `h` wraps to `Closed` then `NeedsYou`; `:818-840` 7 `l` → `Done`, 1 `h` →
`Open` (lists `[Done]`, `[Open, Done]`, `[Done]`); `:949, 993, 1015, 1064` `[Done]`.

### T5.9 Backlog tab (`ui/tabs/backlog/mod.rs`)
`toggle_queued` (`:539-555`), Δ7:
```rust
let state = item.state();
if matches!(state, ItemState::Done | ItemState::Closed) {
    ctx.emit(Action::Error(format!("{} is {}: nothing to queue", item.key, state.word())));
    return;
}
```
Doc `:68`. Tests: `done_only` (`:2237-2241`), `:2454`, `:2538`, `:2567` (`item.state() == Done`),
`with_status` (`:3099`) → `with_state(bench, ItemState)` (call `:3145`); `:3118, 3131, 3150` keep `status:done`.

### T5.10 Runs pane close-out picker (`ui/tabs/backlog/detail/runs.rs`), PD4
```rust
/// ANA-11 §4.2 under MOD-144's law, through `Resolution::closes_from(status, finished)`.
fn resolutions(preview: &Preview) -> Vec<Resolution> {
    Resolution::ALL.iter().copied()
        .filter(|r| r.closes_from(preview.status, preview.finished)).collect()
}
fn cycle(preview: &Preview, forward: bool) -> Resolution { /* as :1101-1112 over resolutions(preview) */ }
// :1142: preview.resolution = cycle(&preview, forward);
```
Tests: `preview()` (`:3781-3790`) `status: Open, finished: true`; `:3875` `Open, finished: false`
(doc: "an `open` item with no finished run closes as `withdrawn`"); `:3963`, `:3978` `Blocked, finished:
false`; `an_open_preview_starts_on_withdrawn` (`:3990-4010`): `default_for(None) == Withdrawn`, open
and blocked unfinished resolve alike, `resolutions(&Preview { status: Closed, .. })` empty (replaces
`Queued`), `cycle` both ways → `Rejected`; new `a_blocked_finished_preview_offers_all_six` (PD18).

### T5.11 Requirements coverage (`ui/tabs/requirements/detail.rs`)
`:123`, `:154` `row.item.state()`; doc `:10` "a citing item's state". Tests `:293, 304` (`Open` +
`Some(AwaitingApproval)`), `:395-397` (`(Open, Some(Running))`, `(Open, Some(AwaitingApproval))`),
`:405-406` keep `◐ running`, `⚠ needs you`; `:408` raw list adds `needs_you`.

### T5.12 Queue overlay and overview
- `ui/overlay/queue.rs`: `row()` literal (`:718-736`) `latest_run_status: None`; `every_state` (`:831`)
  `Wait::NotReady(ItemState::Done)`; import `:609`; snapshot
  `htui__ui__overlay__queue__tests__states.snap:23` `not ready: item is in_progress` → `not ready: item is done`.
- `src/queue_overview.rs:78`: `.filter(|row| row.state() == ItemState::Open && !ready.contains(&row.entry.item_id))`.
  Tests `:436, 438, 464` → `Status::Open` (runs carry the state); `:501`, `:764` `Wait::NotReady(..)`
  carries the `ItemState` T1's fixture gives that row — re-derive it (FEAT-1 now has a live RUN_4, so
  if the row is FEAT-1 it classifies `Running`/`Elsewhere` before `NotReady`; pick a row whose state is
  not live, e.g. ANA-1 `Done`); comment `:412-414`; `:792` stays.

### T5.13 Other htui source and tests
- `src/app/update.rs:702-715`: `ItemSummary` literal appends `latest_run_status: None`.
- `src/store_worker.rs` (PD17): `:4067-4073` `states: Some(vec![ItemState::Done])`; literal
  `:4137-4150` appends the field; `read_items` (`:2583`) spreads `..filter.clone()` (no change).
- `src/run_worker.rs:4830-4835`: assert `item.status == Status::Open` and the run `Queued` (or the
  state via `store.items(..)` → `ItemState::Queued`). `:987`, `:2276` stay.
- `src/mcp_search.rs`, `requirements.rs:808`: unchanged.
- No htui source calls `start_enabled`/`unblock_enabled`/`close_out_enabled`/`default_for` (verdicts
  come from htui-worker); only `runs.rs:53`, `:1959` docs and test `:3994`.
- `tests/runs_pg.rs` (Pg; harness applies 0023): `:1094, 1114` `Status::Open` + run `AwaitingApproval`;
  `:1125` `Open` (run `Done` asserted `:1122`); `:1159` `(Status::Open, None)` (Δ10); `:1239-1242` keep
  `assert_ne!`, reword "set by close-out, never before"; `:1365` FollowRun → `Open`, run
  `AwaitingApproval`; note `:1375` follows T3's FollowRun sentence; docs `:2`, `:1303`; `:1521` `Open`;
  `free_feat_3` doc `:761-763`.
- `tests/backlog.rs`: `:104-107` `frame.contains("⚠ needs you")`; `:1384` `"▸ FEAT-1 ◐ running"`;
  `:3551, 3563` `"FEAT-1  FEAT  ◐ running"`; `TICK_DONE` (`:1706`) 7 `l` presses (fix the array type
  length); `:1728`, `:1878` keep `status:done` with T1's count (ANA-1, agy ANA-1); `:3414` stays `"ANA-1
  is done: nothing to queue"`; doc `:1496`.

### T5.14 Snapshots (225 htui `.snap` today)
**Expected to move** (review lens: only status words/glyphs and the widths they imply; plus the
top-bar active-run count, which T1's demo raises — Platform `on_top_bar` 1 → 3 with RUN_2, RUN_4, RUN_5):
`backlog__*` (~40 of 41), `shell__about_panel_on`, `shell__after_switch`, `shell__switcher_open` (and
`shell__migration_prompt`, `shell__offline_label`, `shell__switcher_empty` only if they show rows or the
top bar), `waiting__platform_mixed`, `waiting__graphics_empty`, `integration__demo_shell`,
`replay__runs_step_selected`, `prompt_preview__preview_ana_2`, `prompt_preview__preview_feat_1`,
`concepts_search__*` (5), `htui__ui__overlay__queue__tests__states` (`:23` only),
`requirements__*` only if FEAT-1's derived state changes (it stays `◐ running` under T1's fixture, so
expect none). Plus T4's 2 and T1's prompt snapshots.
**Must not move**: `htui__ui__overlay__leave__*` (4), `chat__*` (16), every settings family
(`settings`, `box_settings`, `connection`, `hierarchy`, `kinds`, `personas`, `probe`, `project_types`,
`prompt_settings`, `qdrant_settings`, `queue_settings`, `secrets_settings`, `skills`, `templates`),
`state_cli_pg__*`, `queue_overlay__opened`, the other 7 `htui__ui__overlay__queue__tests__*`,
`htui__app__pane__*`, `htui__ui__editor_pane__*`, `htui__ui__overlay__waiting_list__*`,
`htui__testkit__tests__shell_empty`. **No** snapshot moves for `■` (PD14). Runs-tab snapshots that list
the new demo runs (RUN_4..RUN_8) may move; each must show only the added rows.

### T5.15 Commit groups (htui is red between 1-3; commit in order)
1. `feat(MOD-144): kit and theme keyed by ItemState; STOPPED glyph ■` — theme.rs, kit/status.rs, kit/label.rs, overlay/leave.rs.
2. `feat(MOD-144): Backlog list, body, graph and status facet render ItemState` — list.rs, body.rs, graph.rs, filter.rs, mod.rs.
3. `feat(MOD-144): close-out picker, coverage and queue overview on run facts` — runs.rs, requirements/detail.rs, overlay/queue.rs, queue_overview.rs, app/update.rs, store_worker.rs, run_worker.rs.
4. `test(MOD-144): htui integration tests on the three-value status` — tests/backlog.rs, tests/runs_pg.rs.
5. `test(MOD-144): re-accept snapshots (status words and glyphs only)` — snapshots only.

### T5.16 Verification
```bash
cargo check -p htui --all-features
cargo test -p htui --all-features -- --test-threads=1        # without --all-features tests/*.rs run 0 tests
cargo insta test -p htui --all-features -- --test-threads=1   # then diff every .snap.new by eye
cargo insta pending-snapshots && cargo insta accept
cargo clippy -p htui --all-features -- -D warnings
cargo clippy -p htui -- -D warnings
```
`runs_pg.rs`, `worker_pg.rs`, `state_cli_pg.rs` need `HTUI_TEST_DATABASE_URL`; without it they pass vacuously.

---

## T6 — docs and gate (last)

- `README.md:565-571`: replace the two collection paragraphs with: "Run `--index-items` again to pick up
  changes. When an upgrade changes what the index stores for each point, htui drops its collection
  (`htui_concepts_v2`) on first use and the next `--index-items` rebuilds it; nothing needs deleting by
  hand." and keep today's embedder paragraph ("If a later htui embeds with another model, it refuses
  the old index and says so; delete it with `curl -X DELETE
  http://localhost:6333/collections/htui_concepts_v2` and run `htui --index-items` again.").
- `CONCEPTS.md`, `docs/REQUIREMENTS.md`: no edit (PD15).
- `HANDOFF.md:48-60` live coordinates: migrations through `0023_item_status` (MOD-144: three-value item
  status, `uq_run_item_live`), **next `0024`**; cache stays through `0007_project_type`, next `0008`;
  drop MOD-144 from the "still owe one" list (`:57`); core `CASES` 206 → 215, orch `CASES` 109 → 112;
  `TABLES` 51. `:1071` (MOD-119): "key on `htui_core::model::ItemState` (`ALL` is the attention order;
  `ItemFilter.states`); MOD-144 rekeyed the facet, list, detail and graph marks, not grouping or `h`".
  `:394-404` (MOD-135): "builds on `Resolution::closes_from(status, finished)` and `default_for` from the
  run's `graph_snapshot`; MOD-144 landed". `:461-466` (MOD-150): remove "Blocked on MOD-144." and recount
  the summary table. The MOD-144 checklist line (`:419-428`) is handoff-run's close-out bookkeeping, not T6's.
- Workflow-doc rules (`.claude/rules/workflow-docs.md`): HANDOFF holds outstanding work only; live
  coordinates stay in its recap lines; any open-item count change updates the summary table; no
  ID minting by grep (none needed); rule files are not touched.
- Commit: `docs(MOD-144): README index wording; HANDOFF coordinates and MOD-119/135/150 notes`.
- Verify:
  ```bash
  bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
  bash scripts/gate.sh                      # fmt, clippy-all, clippy-bare, postgres, test (--no-fail-fast, --test-threads=1), abort-grep, migrations (0001..0023 contiguous), workflow-docs, plugin
  cargo clippy --workspace -- -D warnings   # featureless (also the gate's clippy-bare)
  ```
  After the test step read `target/gate/test.log`'s abort-grep (SIGABRT; "overflowed its stack" in
  `pg_conformance`); re-run `qdrant_live` timeouts serially before calling them regressions.

---

## Hazards (all tasks)

- **Cross-store drift (Pg / cache / Mem).** Three spellings of LATEST / UNRELEASED / READY / STATE.
  Guards: conformance on Mem and Pg (trait-reachable rules), Pg-vs-Mem parity tests for the inherent
  queue methods (PD5), `mirror_reads_equal_the_reference_store` + `the_mirror_derives_item_state_like_postgres`
  (cache), `item_state_case_matches_item_state_of` (SQL CASE vs `ItemState::of`). Never compare against
  `item.status` alone in a readiness clause.
- **`kind = 'graph'` in every latest-run lookup.** Chat runs carry `item_id` (`pg_criteria.rs:10765`).
  The `close_out` live probe and `waiting_candidates`' `awaiting_approval` EXISTS stay any-kind, as today.
- **`stopped` in every live set** (B5): `uq_run_item_live`, `create_run`'s live read (Mem
  `live_graph_run`, Pg query), `startable` callers (`start_enabled`, `enqueue_with`), `close_out`'s
  probe, the migration pre-check. Conformance case 1 and orch `a_stopped_run_holds_its_item` pin it.
- **`closed_at` only on close-out** (B7): Mem `transition` `:2244`, Pg `transition` `:1923` stop writing
  it; `promote_step`'s clear goes with its item write; 0023 clears it on non-closed rows.
- **`queue_rows` has two latest runs**: `lr` keeps cancelled runs (`QueueRunFact`, `Escalation::Failed`
  only when that run is `failed`), `ls`/`latest_run_status` skips them (`ItemState`). Do not merge them.
- **PD1 is the one regression trap in orch**: without `startable` first, `refuse_missing_tags`/
  `refuse_rung_four` block an `open` item whose run is live.
- **Demo fixture coupling**: T2 (`load_demo` must pass `chk_item_status` and `uq_run_item_live`), T3/T5
  expectations for FEAT-1, FEAT-3, TOOL-1, CLEAN-1, ANA-1 come from T1's table — read it, do not
  invent states. Live demo runs carry `executing_box_id: None` (box slots, `adopt_runs`).
- **Forbidden literal**: the z-word that htui-agent's extensibility test refuses must not appear in any
  new name, comment, tool description, note or fixture string; only the workspace gate catches it.
- **Workspace not compiling between T1 and T5**: per-crate `-p` builds only (build matrix above);
  `Preview.finished` (T3) and `runs.rs` (T5) must both land before htui builds.
- **Migration error text**: item keys in the `RAISE` message (not DETAIL); test 2 pins it.
- **Decision B race**: two processes may rebuild at once; a failed `create_collection` on a now-current
  collection is accepted once, never looped.
- **Snapshot churn**: every moved snapshot swaps words/glyphs/widths (or adds demo runs) only; the
  reviewer diffs the snapshot-only commits in isolation.
