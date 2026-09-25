# MOD-38 — Requirements schema, seam and close-out resolution

> Routed as **PRD** by `/handoff-run MOD-38` (criteria C2, C3, C4 fired; accepted by the maintainer
> 2026-09-25). Ultracode recommended for the implement phase (C4, ≥3 criteria). `docs/ANA-11.md`
> is the design: §5 fixes the schema, §5.1 the seam, §5.2 the cache. This document records what the
> tree says about landing that design, the forks ANA-11 left open, and the milestones.
> Requirements: `R-ENT-8`, `R-ENT-14`, `R-ENT-15`, `R-NF-4`, `R-STO-3`, `R-TUI-9` (applied
> 2026-09-25 by maintainer decision on ANA-11 §7; `R-MCP-2`'s `requirement_cite` deferred to MOD-11).

## Problem

`R-ENT-14` and `R-ENT-15` now require requirement rows with a revision trail and suspect-aware
citations from items, and `R-ENT-8` requires every closed item to carry a resolution. The store has
none of it. There is no requirement table, type or column anywhere in `crates/**`, and no
`resolution` column, type or field: every close-out lands in `closed`, so `done`, `concluded`,
`rejected`, `withdrawn` and `superseded` are indistinguishable. Worse, an item withdrawn or
rejected before it ever ran (MOD-17..19 under MOD-25, an ANA rejected at triage) has no legal path
to `closed` at all: `Status::can_move_to` refuses `open → closed`.

MOD-39 (Requirements tab), MOD-8 (importer, widened by ANA-11) and MOD-34's `resolution` payload and
requirement indexing all wait on this item.

## Evidence

Read at `d966413` (main) during routing. Paths are relative to `crates/`.

- **Seam.** `ReadStore` (16 methods) and `WriteStore: ReadStore` (66 methods) live in
  `htui-core/src/store/traits.rs:66-183` and `:195-1058`, with no default bodies. `close_out` is
  `traits.rs:1045`: `async fn close_out(&self, item, summary: NewDocument, commits: &[RunStepCommit])
  -> Result<Document>`. `UpdateOutcome` (`:1336-1349`) is typed to `Item`/`ItemRevision`;
  `CasOutcome<T>` (`:1357`) is the generic CAS shape.
- **Implementors.** `PgStore` (`htui-store/src/pg/write.rs:3902`), `MemStore`
  (`htui-core/src/store/mem.rs:4065`, wrapper `:4761`), `Writer` (`htui-store/src/writer.rs:879`),
  and two pass-through test doubles (`htui-agent/src/conformance.rs:674`,
  `htui-agent/tests/recorder.rs:354`). `CacheStore` and `Backend` implement reads only. Every new
  trait method is therefore 6 files (write) or 8 files (read, adding `pg/read.rs`, `cache/read.rs`
  and `backend.rs`'s three-arm forwarding).
- **The close-out guard is the shared state machine.** Both stores call
  `legal_move(status, Closed)` (`pg/write.rs:3957`, `mem.rs:4096`), i.e. `Status::can_move_to`
  (`htui-core/src/model/item.rs:50-64`): blocked/failed/done → closed, nothing else. The same rule
  lets the **generic** `WriteStore::transition` reach `closed` too, and three existing cases rely on
  it (`conformance.rs:724` `no_delete_path`, `:6388` `illegal_transitions_are_constraint`,
  `htui/tests/pg_criteria.rs:553`). Under ANA-11's `item_resolution_iff_closed` CHECK those would
  fail on Postgres and pass on MemStore. See Q1.
- **Callers.** Production reaches the store once, `htui-orch/src/engine.rs:1493`, from
  `Command::CloseOut { item }` (`command.rs:130`), which the TUI sends from
  `htui/src/ui/tabs/backlog/detail/runs.rs:716` and `htui/src/run_worker.rs:2055`.
  `close_out_enabled` (`command.rs:1165`) is the UI-side mirror of the guard. See Q2.
- **Key minting precedent.** `item.key` is a `GENERATED ALWAYS … STORED` column
  (`migrations/0001_init.sql:315`), and `mint_item` (`pg/write.rs:398-461`) is one
  `INSERT … ON CONFLICT DO UPDATE … RETURNING` CTE over `item_key_counter`, with the counter row
  created lazily. ANA-11 §5.1 says "`UPDATE … RETURNING`"; the requirement mint follows the real
  pattern instead.
- **Triggers.** `set_updated_at` is attached by a loop over 20 tables in `0001_init.sql:574-580`,
  BEFORE UPDATE only. Migrations 0002-0004 add none; 0005 attaches its own.
- **Migration pins.** `htui-store/tests/migrations.rs` asserts `vec![1, 2, 3, 4]` (`:72-78`,
  `:591`), `TABLES.len() == 33` (`:95`), and exactly 25 commented columns (`:380-409`). sqlx runs
  offline (227 files in `htui-store/.sqlx/`), so changed queries need `cargo sqlx prepare`.
- **Cache.** `CacheStore::open` rebuilds when `cache_meta.schema_version` differs from the
  **Postgres** `schema_version` (`htui-store/src/cache/mod.rs:127-160`), so 0005 alone rebuilds
  every cache. The refresher (`cache/refresh.rs:232`, spawned from `htui/src/store_worker.rs:1787`)
  walks a hardcoded table list with one match arm per table; `refresh_item_link` (`:903-963`) is
  the tombstone-becomes-delete precedent. `MIRRORED_TABLES` has 17 entries (`cache/mod.rs:40`).
- **Demo fixture.** `htui-core/src/fixtures.rs` holds 13 items, loaded by `MemStore::demo()` and
  `PgStore::load_demo` (`htui-store/src/pg/demo.rs:47`) through direct INSERTs. Exactly one is
  closed: `FIX-1` (`fixtures.rs:900-911`). The migration backfill does not reach it, because the
  demo loads after migrating, so the fixture must carry a resolution itself.
- **Suites.** The store conformance suite (`htui-core/src/store/conformance.rs`, 53 write + 9 read
  cases) runs on MemStore, PgStore and (reads) the cache, pinned at `htui-core/tests/mem_store.rs:36,47`
  and `htui-store/tests/pg_conformance.rs:19`. The orchestrator suite (70 cases) runs on MemStore
  only, so close-out with a resolution is proved on Postgres only by `htui/tests/runs_pg.rs`.
- **Project delete.** Requirement tables cascade from `project`, so `DeleteReach`
  (`traits.rs` ~`:1393`) and its producers (`pg/write.rs:318`, `mem.rs:2854`,
  `htui/src/hierarchy.rs:483`) must count them, or the delete confirmation under-reports.

## Users

- **The maintainer**, who decides requirements and closes items. After MOD-38 a withdrawn item can
  close, and every closed item says how it ended. The UI for requirements is MOD-39's.
- **MOD-39, MOD-8, MOD-11 and MOD-34**, which build on this seam: the tab, the importer, the MCP
  `requirement_cite` tool, and the Qdrant payload.

## Hypothesis

If requirements are typed rows cited by items with a version stamp, and resolution is a column set
only by close-out, then MOD-39 and MOD-8 can be built as pure consumers, and "which items address
R-X, and which of them were written against an older text" is a query rather than a grep.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Resolution invariant | `(status = 'closed') = (resolution IS NOT NULL)` holds on both stores, for every path into `closed` | CHECK on Postgres; the same rule asserted by a conformance case on MemStore and PgStore |
| Early withdrawal | An `open` item closes with `rejected`/`withdrawn`/`superseded`/`duplicate`; `done`/`concluded` still need `done`; a live run still refuses | Conformance cases on both stores and `runs_pg.rs` |
| Orchestrator cannot jump | `transition(open → closed)` stays refused | Existing `illegal_transitions_are_constraint` kept green |
| Key minting | `R-<AREA>-<N>` minted per project and area, never reused, race-free | Concurrent-mint case on Postgres mirroring the item-key one |
| No silent overwrite | Amend and withdraw are version CAS and write a revision naming the deciding item | Conformance cases (stale version diverges, revision row present) |
| Suspect is derived | A citation stamped at v1 reads `suspect` after an amend to v2, and not after `reconfirm` | Conformance case on all three read paths (Mem, Pg, cache) |
| Offline reads | Areas, requirements, citations and resolutions are readable from the cache | `htui-store/tests/cache.rs` `run_all_reads` |
| Demo | The demo carries a small requirement set and one suspect citation, and FIX-1 has a resolution | `load_demo` row-delta test (`migrations.rs:1103-1145`) and `MemStore::demo()` |

## Scope

**In scope**

- `migrations/0005_requirements.sql` per ANA-11 §5: `requirement_spec`, `requirement_area`,
  `requirement_key_counter`, `requirement` (generated key), `requirement_revision`,
  `item_requirement`, `item.resolution` with the backfill `closed → done` before
  `item_resolution_iff_closed`, and `set_updated_at` triggers on the four mutable tables.
- `cache_migrations/0004_requirements.sql` per §5.2: `requirement_area`, `requirement`,
  `item_requirement` (live rows only) and `item.resolution`; refresher arms for the three new
  tables; `MIRRORED_TABLES` and `ITEM_COLUMNS` updated.
- A `Resolution` `str_enum!` with a DDL-agreement test like `status_matches_check_list`, and
  `Item.resolution: Option<Resolution>` through every `Item` select and literal.
- `close_out(item, resolution, summary, commits)` on the trait and every implementor, with its
  own guard (below), and the orchestrator plumbing of Q2.
- Requirement models and the §5.1 methods: reads on MemStore, PgStore and the cache; writes on
  MemStore and PgStore (no offline write, per MOD-25).
- Conformance cases for all of the above on both stores, plus cache read cases.
- `DeleteReach` counts requirement rows.
- Demo fixture: a small requirement set, one suspect citation, and a resolution on FIX-1.
- Close-out write-up recording the ANA-2 §4.3 amendment (open → closed via close-out for
  non-success resolutions).

**Out of scope**

- Any TUI surface: the Requirements tab, suspect markers, re-confirm action and resolution picker
  are MOD-39.
- MCP `requirement_cite` (MOD-11), the prompt `requirements` section (MOD-9 or a follow-up, §5.3).
- Importing `docs/REQUIREMENTS.md` or `DECISIONS.md` (MOD-8).
- Qdrant indexing of requirements (MOD-34's follow-up note).
- Role enforcement for "maintainer-only" writes: the store has no role model; the TUI (MOD-39) and
  the MCP surface (MOD-11) enforce who may call what, and agents never get `amend_requirement`.

## Constraints (fixed before planning)

- **ANA-11 §5 is the schema.** Deviations are recorded in the plan with a reason. The mint follows
  `mint_item`'s CTE pattern, not the "`UPDATE … RETURNING`" phrasing of §5.1.
- **Resolution legality**, enforced in `close_out` and not in `Status::can_move_to`:
  `done`/`concluded` only from `done`; `rejected`/`withdrawn`/`superseded`/`duplicate` from `open`,
  `blocked`, `failed` or `done`; everything else refused. A live run refuses first, as today.
- **Suspect is derived** (`requirement.version > item_requirement.requirement_version`), never
  stored.
- **Citing a withdrawn requirement** is refused for `addresses` and `reserves`; `amends` and
  `withdraws` remain legal as history.
- **One migration pair.** 0005 and cache 0004 ship together. If MOD-7 lands a migration first,
  this item renumbers at merge.
- **Forward-only, sqlx offline data regenerated, migration pins updated rather than deleted.**
- `unsafe_code = "forbid"`, workspace lints unchanged, TDD per repo convention; reviewer is
  `rust-reviewer` (`.claude/workflow-config.json`).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Resolution | Every closed item says how it ended; an early-withdrawn item can close; the orchestrator still cannot jump to `closed`. Migration (resolution part), `Resolution` type, `close_out` guard, orchestrator plumbing, FIX-1, cache column. | pending | — |
| 2 | Requirements | Areas, requirements and revisions exist with minted keys, CAS amend and withdraw naming the deciding item, and the spec header; readable on both stores. | pending | — |
| 3 | Citations and offline | Items cite requirements, suspect is derived and re-confirmable, coverage is a query, the cache mirrors all of it, project delete counts it, and the demo shows one suspect citation. | pending | — |

Milestone 1 is independently useful (it is what MOD-34's `resolution` payload waits for) and
touches the orchestrator; milestones 2 and 3 are store-only. All three ship in one branch because
they share one migration.

## Open Questions

- **Q1 — How does the generic `transition()` treat `closed`?** Options: (a) `closed` becomes
  reachable **only** through `close_out`; `transition(_, _, Closed)` is refused, and the three
  cases that use it to reach `closed` switch to `close_out`. (b) `transition` into `closed` stamps
  `resolution = 'done'` implicitly. **Recommended: (a).** It matches `R-ENT-8` ("driven … by
  close-out") and ANA-11's invariant that resolution is set only by close-out; (b) would let an
  item close as `done` from `failed`.
- **Q2 — Where does the resolution come from before MOD-39's picker exists?** Options:
  (a) `Command::CloseOut { item, resolution }`; the Runs pane and `run_worker` send a default
  derived from status (`done` → `done`, `blocked`/`failed` → `withdrawn`) until MOD-39 replaces
  it. (b) `Command::CloseOut` unchanged and the engine derives the same default, so MOD-39 changes
  the command later. (c) Pull a minimal resolution picker into MOD-38. **Recommended: (a).** The
  command shape is settled once, MOD-39 only swaps the source, and the TUI change is one argument.
- **Q3 — Does amending or withdrawing a requirement also record the citation?** Options: (a)
  `amend_requirement`/`withdraw_requirement` write the revision **and** upsert the deciding item's
  `amends`/`withdraws` citation, stamped at the new version, in one transaction. (b) The caller
  cites separately. **Recommended: (a).** The revision already names the item; a separate call can
  be forgotten and leaves coverage and history disagreeing.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The CHECK makes Postgres and MemStore disagree on paths into `closed` | High without Q1 | High | Q1's rule enforced in shared Rust code and asserted by one conformance case on both stores |
| Migration collision with MOD-7 (run in parallel) | Medium | Low | One migration pair; renumber at merge; the pins in `migrations.rs` make a collision loud |
| The cache rebuild on 0005 surprises a user with a large cache | Low | Low | Existing behaviour on every PG migration; nothing new |
| Wide mechanical change (≈9 files for `close_out`, 6-8 per new method, sqlx data) invites a missed implementor | Medium | Medium | No default trait bodies, so the compiler finds every one; ultracode implement phase fans out by file set |
| Merge churn with MOD-34 in `HANDOFF.md`, `REQUIREMENTS.md`, `Cargo` files | High | Low | Small textual merges; MOD-34 consumes `item.resolution` rather than defining it |
