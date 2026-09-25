# Plan: MOD-38 requirements schema, seam and close-out resolution (milestones 1–3)

**Source PRD**: `.claude/prds/mod-38-requirements-schema.prd.md`. The maintainer answered the gate
decisions D1–D3 on 2026-09-25; this plan implements them and does not re-open them.
**Selected milestones**: all three. They share one migration pair (`0005_requirements.sql`,
cache `0004_requirements.sql`) and one branch.
**Numbering**: the PRD owns **D1–D3**. This plan's decisions start at **D4**, and tasks at **T1**.

**Design authority**: `docs/ANA-11.md` §4.2 (resolution), §4.3 (citations), §5 (schema), §5.1
(seam), §5.2 (cache); `docs/ANA-2.md` §4.3 (the item status table this item amends) and §4.10
(close-out). Requirements: `R-ENT-8`, `R-ENT-14`, `R-ENT-15`, `R-NF-4`, `R-STO-3`, `R-STO-5`,
`R-TUI-9`.

**Complexity**: large but mechanical. There is no new crate and no new dependency. The width comes
from the seam: `ReadStore` and `WriteStore` have no default bodies, so every method lands on
MemStore, PgStore, `Writer`, `Backend`, the cache and two test doubles.

**Routing**: PRD path (C2, C3, C4). Ultracode is recommended for implement: the fan-out is
T4 ∥ T5 and T7 ∥ T8 ∥ T9, followed by an adversarial-verify pass. The review gate is
`rust-reviewer` (`.claude/workflow-config.json`).

## Summary

Milestone 1 makes resolution real. It adds a `Resolution` type and an `item.resolution` column set
only by `close_out`. It adds a close-out law separate from the orchestrator's status table:
`closed` stops being a `transition` target (D1), and `close_out` accepts `open` for the four
non-success resolutions. Finally it threads a resolution through `Command::CloseOut`, with a
status-derived default until MOD-39 (D2).

Milestones 2 and 3 add ANA-11 §5's requirement tables behind the §5.1 methods. MemStore and
PgStore get the writes, all three stores get the reads, and the cache mirrors areas, requirements,
live citations and the spec header. The demo fixture shows one suspect citation.

## Design decisions (settled here)

- **D4 — `Status::can_move_to` loses its three `→ closed` edges.** The edges out of `blocked`,
  `failed` and `done` are removed, and the close-out law moves to `Resolution::closes_from(self,
  status)`:
  - `done` and `concluded` close only from `done`;
  - `rejected`, `withdrawn`, `superseded` and `duplicate` close from `open`, `blocked`, `failed` or
    `done`;
  - everything else is refused.

  `SANCTIONED` in `model/item.rs` drops the same three rows. A new transcribed table,
  `CLOSE_OUT_SANCTIONED`, pins the close-out law the same way. `legal_move(_, Closed)` is then
  refused on every store, which is D1.
- **D5 — MemStore's `close_out` sets the status directly.** It no longer calls its inner
  `transition` (`mem.rs:4110`), because D4 makes that refuse. This matches Pg's direct
  `UPDATE … SET status = 'closed'` (`pg/write.rs:3987`). The guard order stays the same on both
  stores:
  1. NotFound;
  2. a live run;
  3. the summary's kind;
  4. the summary's item;
  5. `closes_from`;
  6. the commit steps.
- **D6 — Default resolution.** `Resolution::default_for(Status) -> Option<Resolution>` maps `done`
  to `Done`, `blocked`/`failed` to `Withdrawn`, and anything else to `None`.
  - `close_out_enabled` refuses when it answers `None`, so the Runs pane keeps greying `c` on an
    `open` item. The store accepts `open` from here on, but the TUI offers it only with MOD-39's
    picker.
  - `closeout::Preview` gains `resolution`, and the Runs pane sends exactly that value in
    `Command::CloseOut { item, resolution }`.
- **D7 — Resolution is a column, not text.** The `summary` document's body does not change.
- **D8 — One `0005_requirements.sql`, written whole in T2.** It follows ANA-11 §5, with these
  deltas:
  - the counter is minted with `mint_item`'s `INSERT … ON CONFLICT DO UPDATE … RETURNING` CTE
    (`pg/write.rs:398-461`), not the "`UPDATE … RETURNING`" of §5.1;
  - `set_updated_at` triggers are attached to `requirement_spec`, `requirement_area`,
    `requirement` and `item_requirement` by explicit `CREATE TRIGGER`s;
  - the backfill is `UPDATE item SET resolution = 'done' WHERE status = 'closed'` before
    `item_resolution_iff_closed`.

  There are no `COMMENT ON COLUMN`s, so `ANA_COLUMN_COMMENTS` stays at 25.
- **D9 — The requirement amend outcome is its own enum.** It is
  `RequirementUpdate { Updated(Requirement), Diverged { head: Requirement, ancestor:
  RequirementRevision } }`. `UpdateOutcome` is typed to `Item` (`traits.rs:1336`) and is not
  generalised. `set_requirement_spec` returns `CasOutcome<RequirementSpec>`, because the spec keeps
  no revision history.
- **D10 — Citation rules.**
  - `cite` stamps `requirement.version` at the time of the call, and upserts, reviving a
    tombstone.
  - `addresses` and `reserves` citations of a `withdrawn` requirement are refused with
    `Constraint`.
  - `uncite` tombstones the row. `reconfirm` re-stamps it, and a missing or tombstoned row is
    `NotFound`.
  - `amend_requirement` and `withdraw_requirement` upsert the deciding item's `amends`/`withdraws`
    citation at the new version, in the same transaction (D3).
- **D11 — Suspect is computed on read.** Each store computes
  `requirement.version > item_requirement.requirement_version` in its read. It is never stored.
- **D12 — Cache contents.** The cache mirrors `requirement_area`, `requirement`,
  `item_requirement` (live rows only; a tombstone deletes, like `refresh_item_link`),
  `item.resolution` **and** the one-row `requirement_spec`.
  - Mirroring the spec is a deviation from §5.2. It is one small row per project, and it removes
    an ambiguous "none or not cached" answer from `requirement_spec`.
  - Revisions are not mirrored. `requirement_revisions` returns `Result<Option<Vec<…>>>`, where
    `None` means "not cached", following `step_events` (`traits.rs:80`).
- **D13 — `DeleteReach` counts the new rows.** It gains `requirement_areas`, `requirements` and
  `item_requirements` (citations whose item or requirement is in the project). The producers
  (`pg/write.rs:318`, `mem.rs:2854`) and the confirmation text (`htui/src/hierarchy.rs:483`)
  follow.
- **D14 — `RequirementFilter { area_codes, states, priorities, text }` mirrors `ItemFilter`.**
  Every field is a conjunct and `None` means "don't filter". Text matching works like `items`:
  `position`/`contains`/`instr` over key and body.
- **D15 — IDs.** `RequirementId` and `RequirementAreaId` come from `id_newtype!`
  (`model/ids.rs:15`). Requirement models live in a new `model/requirement.rs`.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Enum with DDL agreement | `model/item.rs:8-28`, `model/mod.rs:147-167` | `str_enum!` + `check_enum` test against the migration's CHECK list |
| Generated key + lazy counter | `0001_init.sql:299-315`; `pg/write.rs:398-461`; `mem.rs:1197-1257` | one CTE: counter upsert, row insert, revision insert |
| CAS with divergence | `pg/write.rs:486-531` (`update_item`) | `WHERE version = $n`, on miss read head + ancestor revision |
| Tombstoned edge | `0001_init.sql:357-369`; `pg/read.rs:182`; `cache/refresh.rs:903-963` | `deleted_at`, `WHERE deleted_at IS NULL`, cache deletes on tombstone |
| Filter struct | `model/item.rs:160-176`; `pg/read.rs:67-126` | `($n::type IS NULL OR …)` binds |
| Conformance case | `store/conformance.rs:37,102,196` | name in `CASES`, arm in `run_case`, pins bumped in `mem_store.rs`/`pg_conformance.rs` |
| Cache refresh arm | `cache/refresh.rs:232-288, 820-852` | explicit table list, one arm per table, cursor on `updated_at` |

## Files to Change

Paths relative to `crates/`.

| File | Task | Why |
|---|---|---|
| `htui-core/src/model/item.rs`, `model/mod.rs` | T1 | `Resolution`, `Item.resolution`, D4 tables, `check_enum` test |
| `htui-core/src/fixtures.rs`, `store/mem.rs`, `htui-orch/src/closeout.rs`, `htui-store/src/cache/read.rs`, `htui-core/src/prompt/{fixtures,render}.rs` | T1 | `Item` literal sites gain `resolution` (the compiler enumerates them; FIX-1 → `Done`) |
| `htui-store/migrations/0005_requirements.sql` | T2 | new, whole (D8) |
| `htui-store/src/pg/read.rs`, `pg/write.rs` (item selects), `pg/demo.rs` (item insert) | T2 | `resolution` through every item select and the demo insert |
| `htui-store/tests/migrations.rs`, `htui-store/.sqlx/*` | T2 | version vecs → `[1..=5]`, `TABLES` 33 → 39, the 0004 test runs to 4 explicitly; regenerated query data |
| `htui-core/src/store/traits.rs` (`close_out`), `pg/write.rs` (`close_out`), `mem.rs` (`close_out`, tests), `htui-store/src/writer.rs`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs` | T3 | new signature, D4/D5 guard |
| `htui-core/src/store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`, `htui/tests/runs_pg.rs`, `htui-store/tests/pg_criteria.rs` | T3 | close-out matrix case, "transition refuses closed" case, the three cases that used `transition` to reach `closed`, pins |
| `htui-orch/src/command.rs`, `engine.rs`, `closeout.rs`, `conformance.rs`, `htui-orch/tests/fake_conformance.rs` | T4 | `Command::CloseOut { item, resolution }`, `close_out_enabled` via `default_for`, `Preview.resolution` |
| `htui/src/run_worker.rs`, `htui/src/ui/tabs/backlog/detail/runs.rs` (+ its snapshots if the preview text changes) | T4 | send the preview's resolution; show "closes as …" in the first confirmation |
| `htui-store/cache_migrations/0004_requirements.sql` | T5 | new, whole (D12) |
| `htui-store/src/cache/mod.rs`, `cache/refresh.rs` (item arm), `cache/read.rs` (item select), `htui-store/tests/cache.rs` | T5 | `item.resolution` mirrored; `MIRRORED_TABLES` grows by 4 |
| `htui-core/src/model/requirement.rs` (new), `model/ids.rs`, `model/mod.rs`, `store/traits.rs` | T6 | models, IDs, §5.1 methods, `RequirementUpdate`, `DeleteReach` fields |
| `htui-store/src/backend.rs`, `writer.rs`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs` | T6 | forwarding arms (complete, not stubs) |
| `mem.rs`, `pg/read.rs`, `pg/write.rs`, `cache/read.rs` | T6 | `unimplemented!()` stubs only, replaced in T7/T8/T9 |
| `store/conformance.rs`, `mem_store.rs`, `pg_conformance.rs` | T6 | the requirement and citation cases, written first and failing |
| `htui-core/src/store/mem.rs` | T7 | MemStore requirements, citations, `DeleteReach` |
| `htui-store/src/pg/read.rs`, `pg/write.rs`, `.sqlx/*` | T8 | PgStore requirements, citations, `DeleteReach` count |
| `htui-store/src/cache/{mod,refresh,read}.rs`, `htui-store/tests/cache.rs` | T9 | mirror and read the four tables |
| `htui-core/src/fixtures.rs`, `htui-store/src/pg/demo.rs`, `htui-store/tests/migrations.rs` (row deltas), `htui/src/hierarchy.rs` | T10 | demo requirement set + one suspect citation; delete confirmation text |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-38.md`, `docs/ANA-2.md` (§4.3 note), PRD, this plan | T11 | close-out bookkeeping |

Deliberately untouched: `htui/src/ui/tabs/settings/*`, box rows, `htui-agent/src/probe*` (all
MOD-7's); every TUI surface for requirements (MOD-39); MCP (MOD-11).

## Milestone → task map

| PRD milestone | Delivered by |
|---|---|
| 1 — Resolution | T1, T2, T3, T4, T5 (the resolution half) |
| 2 — Requirements | T6, T7, T8 (areas, requirements, revisions, spec) |
| 3 — Citations and offline | T6, T7, T8 (citations, coverage, suspect), T9, T10 |

## Tasks

TDD per repo convention: each task opens with the test that fails for the stated reason.
**Independence is decided by file set.** "Needs Tn landed" is a sequencing note, not a file
intersection.

Order: T1 → T2 → T3 → (T4 ∥ T5) → T6 → (T7 ∥ T8 ∥ T9) → T10 → T11.

### T1: `Resolution` and the close-out law (M1; first, alone)
- **Tests first:**
  - `check_enum` for `Resolution` against the 0005 CHECK list. It is transcribed in the test,
    because the migration lands in T2.
  - `SANCTIONED` without the three `→ closed` rows.
  - New `CLOSE_OUT_SANCTIONED` (status × resolution).
  - `default_for` table.
- **Code:** `str_enum!(Resolution …)`, `closes_from`, `default_for`, `Item.resolution:
  Option<Resolution>` (`#[serde(default)]`), and every `Item` literal.
- **Files:** `htui-core/src/model/{item,mod}.rs`, `fixtures.rs`, `store/mem.rs` (literal only),
  `prompt/{fixtures,render}.rs`, `htui-orch/src/closeout.rs` (literal only),
  `htui-store/src/cache/read.rs` (literal only; the value is `None` until T5).
- **Known red after T1:** the conformance cases that `transition` into `closed`. T3 fixes them.
  T1 runs `cargo test -p htui-core --lib`, not the suites.

### T2: Migration 0005 and item selects (M1; needs T1)
- **Tests first:**
  - `migrations.rs`: applied `[1,2,3,4,5]`, `TABLES` +6.
  - `resolution_iff_closed` rejects `closed` with a NULL resolution and non-closed with a value.
  - The backfill sets `done` on a pre-0005 closed row (`run_to(4)`, insert, `MIGRATOR.run`).
  - The 0004 test pins `run_to(4)` rather than "latest".
- **Code:**
  - `0005_requirements.sql` (D8), complete, including the requirement tables.
  - `resolution` in the three item `query_as!` selects and the cache-free reads.
  - `pg/demo.rs` inserts `resolution`.
  - `cargo sqlx prepare -- --all-targets --all-features`.
- **Files:** `migrations/0005_requirements.sql`, `pg/read.rs`, `pg/write.rs` (item selects only),
  `pg/demo.rs`, `tests/migrations.rs`, `.sqlx/*`.

### T3: `close_out(item, resolution, …)` on the seam (M1; needs T2)
- **Tests first:**
  - Conformance case `close_out_resolution_law`: every status × resolution against
    `closes_from`, with `open` + `withdrawn` succeeding, `open` + `done` refused, and resolution
    read back.
  - Conformance case `transition_never_reaches_closed`.
  - `no_delete_path` and `illegal_transitions_are_constraint` switched to `close_out`;
    `pg_criteria.rs:553`'s stale-`from` check moves to the legal pair `Done → Open`.
  - `runs_pg.rs` close-out asserts the resolution.
- **Code:** trait signature and doc (the ANA-2 §4.3 amendment in the doc comment), Pg guard and
  `UPDATE … SET status='closed', resolution=$n`, Mem D5, `Writer`, the two doubles, and the engine
  call site passes a placeholder that T4 replaces.
- **Files:** `store/traits.rs`, `pg/write.rs` (`close_out`), `mem.rs` (`close_out` + tests),
  `writer.rs`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`,
  `store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`,
  `htui/tests/runs_pg.rs`, `htui-store/tests/pg_criteria.rs`, `htui-orch/src/engine.rs` (one line),
  `.sqlx/*`.

### T4: Orchestrator and Runs pane plumbing (M1; needs T3; parallel with T5)
- **Tests first:**
  - `command.rs` `close_out_needs_no_live_run_and_a_closable_item` gains `open` → `NotClosable`
    (D6).
  - Orch conformance cases assert `resolution` after `CloseOut`.
  - The engine preview test asserts `Preview.resolution`.
  - A `runs.rs` render test shows "closes as withdrawn" for a failed item.
- **Code:** `Command::CloseOut { item, resolution }`, `Engine::close_out` passes it,
  `close_out_enabled` via `default_for`, `Preview.resolution`, `run_worker` and `runs.rs` send the
  preview's value.
- **Files:** `htui-orch/src/{command,engine,closeout,conformance}.rs`,
  `htui-orch/tests/fake_conformance.rs` (pin, only if a case is added), `htui/src/run_worker.rs`,
  `htui/src/ui/tabs/backlog/detail/runs.rs`, `htui/tests/snapshots/*` (only those that change).
- **∩ T5:** ∅.

### T5: Cache migration and `item.resolution` mirror (M1; needs T2; parallel with T4)
- **Tests first:** `tests/cache.rs` asserts a closed item's resolution after refresh, and
  `MIRRORED_TABLES` has 21 entries.
- **Code:**
  - `cache_migrations/0004_requirements.sql`, complete (D12: the item column plus the four mirror
    tables).
  - The item refresh column list and item read select.
  - `MIRRORED_TABLES` +4. The four new refresh arms are T9's; until then they are listed and
    answer 0.
- **Files:** `cache_migrations/0004_requirements.sql`, `cache/mod.rs`, `cache/refresh.rs` (item arm
  + table list), `cache/read.rs` (item select), `tests/cache.rs`.
- **∩ T4:** ∅.

### T6: Requirement models and the seam, tests first (M2/M3; needs T3, T5)
- **Code:**
  - `model/requirement.rs`: `RequirementArea`, `NewRequirementArea`, `Requirement`,
    `NewRequirement`, `RequirementPatch`, `RequirementRevision`, `RequirementSpec`,
    `RequirementState`, `Priority`, `CitationKind`, `ItemCitation { requirement, kind,
    requirement_version, suspect }`, `CoverageRow { item: ItemSummary, kind, resolution, suspect }`,
    `RequirementFilter`, `RequirementUpdate`.
  - `check_enum` for the four new enums against 0005.
- **Trait (D9–D15):**
  - `ReadStore`:
    - `requirement_spec(project)`
    - `requirement_areas(project)`
    - `requirements(project, &RequirementFilter)`
    - `requirement(id)`
    - `requirement_revisions(id) -> Option<Vec<_>>`
    - `item_requirements(item)`
    - `requirement_coverage(requirement)`
  - `WriteStore`:
    - `set_requirement_spec`
    - `create_requirement_area`
    - `mint_requirement`
    - `amend_requirement`
    - `withdraw_requirement`
    - `cite`
    - `uncite`
    - `reconfirm`
  - `DeleteReach` +3 fields.
- **Forwarding:** `Backend`, `Writer` and the doubles are complete. Mem, Pg and the cache get
  `unimplemented!()` stubs.
- **Cases, written first and red:**
  - `requirement_mint_is_per_area_and_never_reused`
  - `requirement_amend_is_cas_and_names_the_item`
  - `requirement_withdraw_refuses_new_addresses`
  - `amend_records_the_deciding_citation` (D3)
  - `a_newer_version_makes_a_citation_suspect_until_reconfirmed`
  - `uncite_tombstones_and_cite_revives`
  - `coverage_lists_citing_items_with_resolution`
  - `spec_is_cas`
  - `project_delete_counts_requirements`
  - read cases for areas, filter and citations (in `READ_CASES`)
  - pins bumped
- **Files:** `model/{requirement,ids,mod}.rs`, `store/traits.rs`, `store/conformance.rs`,
  `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`, `backend.rs`, `writer.rs`,
  `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`, stub lines in `mem.rs`,
  `pg/read.rs`, `pg/write.rs`, `cache/read.rs`.

### T7: MemStore (M2/M3; needs T6; parallel with T8, T9)
- **Code:** state maps, mint counter keyed by area, CAS + revision, the D10 rules, D11 on read,
  and `DeleteReach`. The goal is the whole suite green on `mem_store.rs`.
- **Files:** `htui-core/src/store/mem.rs` only.

### T8: PgStore (M2/M3; needs T6; parallel with T7, T9)
- **Code:**
  - The mint CTE (D8).
  - Amend: CAS `UPDATE`, revision insert and citation upsert in one transaction; on a miss,
    `Diverged` with the head and its ancestor revision.
  - Withdraw works the same way as amend.
  - Cite, uncite and reconfirm; suspect in SQL; coverage join.
  - The `DeleteReach` count query.
  - `cargo sqlx prepare`.
- **Goal:** `pg_conformance.rs` green, plus one concurrent-mint test mirroring the item-key one.
- **Files:** `pg/read.rs`, `pg/write.rs`, `.sqlx/*`, `htui-store/tests/pg_conformance.rs` (the
  concurrent-mint test only; T6 already bumped the pin).
- **∩ T7:** ∅. **∩ T9:** ∅.

### T9: Cache (M3; needs T6; parallel with T7, T8)
- **Code:**
  - Refresh arms for `requirement_spec`, `requirement_area`, `requirement` and
    `item_requirement`, with the tombstone deleting.
  - The seven reads, with `requirement_revisions` returning `None`.
  - `tests/cache.rs` `run_all_reads` green.
  - A suspect citation read offline.
- **Files:** `cache/refresh.rs`, `cache/read.rs`, `htui-store/tests/cache.rs`.
- **∩ T8:** ∅ (T8 touches `pg/*`, T9 `cache/*`).

### T10: Demo fixture and delete text (M3; needs T7, T8, T9)
- **Code:**
  - Fixture spec header, two areas (`ENT`, `STO`) and three requirements. One of them is amended
    once, so its citation from `HTUI_ANA_1` is stamped at v1 against v2 (suspect).
  - FIX-1 is resolved `done` (done in T1, asserted here).
  - `pg/demo.rs` inserts, `MemStore::demo()`, the `migrations.rs` row-delta test, and the
    `hierarchy.rs` delete confirmation lines.
- **Files:** `fixtures.rs`, `pg/demo.rs`, `tests/migrations.rs`, `htui/src/hierarchy.rs` (+ its
  test/snapshot).

### T11: Close-out (serial, last)
- `docs/decisions/mod/mod-38.md`: what shipped, D1–D15, and the ANA-2 §4.3 amendment (closed only
  through close-out; `open → closed` for non-success resolutions).
- A one-line note in `docs/ANA-2.md` §4.3 pointing at it.
- The `DECISIONS.md` line and HANDOFF P2 close-out:
  - MOD-38 is closed;
  - MOD-39 is unblocked;
  - MOD-8's blocked-on note is updated;
  - MOD-34's follow-up can now read `item.resolution`.
- The PRD milestones are marked complete. The validator must be green.

## Validation

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
pg_ctlcluster 16 main start    # local cluster on this box; HTUI_TEST_DATABASE_URL points at it
HTUI_TEST_DATABASE_URL=postgres://… cargo test --workspace --all-features
(cd crates/htui-store && DATABASE_URL=… cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

`sqlx-cli` is not installed on this box. T2 installs it once (`cargo install sqlx-cli
--no-default-features --features postgres,sqlite`, per `README.md:497-505`).

## Risks

| Risk | Mitigation |
|---|---|
| A store keeps a path into `closed` that bypasses `close_out` | D4 removes the edges from the shared table, and `transition_never_reaches_closed` runs on both stores |
| `unimplemented!()` stubs survive T7–T9 | T11's gate greps for `unimplemented!` in `mem.rs`, `pg/`, `cache/` and fails on a hit |
| MOD-7 lands a migration first | Renumber 0005 → 0006 at merge; `migrations.rs` pins make it loud |
| sqlx offline data drifts between parallel T8 and other `.sqlx` writers (T2, T3) | Those tasks are serial before T8; T8 is the only `.sqlx` writer in its wave |
| `DeleteReach` field growth breaks a snapshot of the delete confirmation | T10 owns the text and re-accepts the snapshot deliberately |

## Verified claims

Checked against the tree at `d966413` (main) on 2026-09-25, before the tasks were written.

| Claim | Verdict | Evidence |
|---|---|---|
| `ReadStore`/`WriteStore` have no default method bodies | true | `htui-core/src/store/traits.rs:66-183`, `:195-1058` |
| `close_out(item, summary, commits)` is the current signature | true | `traits.rs:1045` |
| Both stores guard close-out with `legal_move(status, Closed)` | true | `pg/write.rs:3957`, `mem.rs:4096` |
| MemStore's `close_out` ends by calling its inner `transition`, which re-runs `legal_move` | true | `mem.rs:4110`, `:1372` |
| `can_move_to` has exactly three `→ Closed` edges (blocked, failed, done), transcribed in `SANCTIONED` | true | `model/item.rs:50-64`, `:264-300` |
| Three tests drive an item to `closed` through `transition` | true | `conformance.rs:724`, `:6388`; `htui-store/tests/pg_criteria.rs:553` |
| `pg_criteria.rs:553` uses `transition(Done → Closed)` as its *legal-but-stale* pair (expects `Ok(false)`); under D4 that pair is illegal (`Err`), so the check moves to another legal pair (`Done → Open`) rather than to `close_out` | finding | `htui-store/tests/pg_criteria.rs:551-556` |
| `Command` derives only `Debug, Clone, PartialEq, Eq` (not serialised, so a new field has no wire impact) | true | `htui-orch/src/command.rs:53-54` |
| `Command::CloseOut { item }` and `close_out_enabled` accepting `done/failed/blocked` | true | `command.rs:130-133`, `:1165-1180` |
| `closeout::Preview` has no resolution field | true | `closeout.rs:19-32` |
| `item.key` is a generated column; `mint_item` is one `INSERT … ON CONFLICT DO UPDATE … RETURNING` CTE | true | `0001_init.sql:315`; `pg/write.rs:398-461` |
| `set_updated_at` is attached by a loop in 0001 only | true | `0001_init.sql:574-580`; no trigger in 0002–0004 |
| Migration pins: `vec![1, 2, 3, 4]` twice, `TABLES.len() == 33`, 25 commented columns | true | `htui-store/tests/migrations.rs:74`, `:591`, `:95`, `:381-409` |
| `MIRRORED_TABLES` has 17 entries | true | `cache/mod.rs:40` |
| Cache rebuild keys on the Postgres `schema_version` | true | `cache/mod.rs:127-160` |
| Conformance pins: 53 cases (mem, pg), 9 read cases, 70 orch cases | true | `mem_store.rs:36,47`; `pg_conformance.rs:19`; `fake_conformance.rs:16` |
| The demo holds exactly one `closed` item, FIX-1 | true | `fixtures.rs:900-913` |
| `DeleteReach` counts items and kinds but nothing requirement-shaped | true | `traits.rs:1393-1415` |
| `step_events` is the "`None` = not cached" precedent | true | `traits.rs:80` |
| sqlx runs offline; `cargo sqlx prepare -- --all-targets --all-features` from inside the crate | true | `.cargo/config.toml`; `README.md:495-510` |
| `sqlx-cli` is not installed; Postgres 16 is installed (cluster down) | true | `cargo sqlx` → no such command; `pg_lsclusters` |
| Task independence: T4 ∩ T5 = ∅; T7 ∩ T8 = ∅ (`mem.rs` vs `pg/*`, `.sqlx`); T7 ∩ T9 = ∅; T8 ∩ T9 = ∅ (`pg/*` vs `cache/*`; the pin file is T6's) | true | file sets above |
| **Finding: ANA-11 §5.1's "`UPDATE … RETURNING`" mint does not match the tree's pattern** | finding | D8 follows the tree |
| **Finding: `writer.rs` docs describe a `Buffered` arm and `upload_pending` that no longer exist** | finding | `writer.rs:58-63`; out of scope, noted for a CLEAN item |

## Acceptance

- Every PRD success metric has a named test above.
- The suites are green on MemStore, PgStore and the cache. `cargo clippy -D warnings` and
  `sqlx prepare --check` are clean.
- There is no `unimplemented!` in store code.
- The validator is green, and HANDOFF, DECISIONS and the write-up are updated per T11.
