# Plan: MOD-170 — Waiting key qualifier: one read per refresh tick

**Source**: `HANDOFF.md` MOD-170 (from MOD-147 M2 review). Routed as **plan** by `/handoff-run`
(C3 only; maintainer accepted 2026-11-11).
**Complexity**: Small
**Status**: confirmed by the maintainer 2026-11-11 — implementing

## Summary

The top bar's once-a-second `StoreRequest::Waiting` builds its `KeyQualifier` in `waiting_reads`
(`crates/htui/src/store_worker.rs:2348`) with `scope_project`, then a `project()` and a `kinds_of`
read per scope project, one after another: 2N+1 Postgres round trips a tick online. This plan
replaces them with **one** Postgres query that answers the workspace's scope row and every scope
project's slug and kind prefixes. The mirror and the in-memory store keep the per-project reads
(they are local and cost no round trip). Behaviour does not change: same widened scope, same
qualifier, same refusals.

## Decision: one batched read, not a cache

| Option | Verdict |
|---|---|
| **A. One read** (project slugs + kind prefixes for a set of ids, scope row folded in) | **Chosen.** Always fresh, no invalidation, one round trip. |
| B. Qualifier cached per workspace, cleared on kind and hierarchy writes | Rejected. Kind and hierarchy writes also come from other processes (the `htui` CLI, other boxes, the engine, MCP tools); the store worker only sees its own writes, so the cache would print stale keys until restart. Clearing it on the refresher's notices would couple it to the mirror's change feed for a read that, batched, is already one round trip. |

`refresher_projects` (`store_worker.rs:3139`) caches the scope row id per workspace already; this
plan does not reuse that map, because the batched query answers the scope row for free.

## The read

`PgStore::waiting_keys(workspace: Option<WorkspaceId>, ids: &[ProjectId]) -> Result<ScopeKeys>`
(inherent, `pub`, `crates/htui-store/src/pg/read.rs`, beside `item_kind_rows`):

```sql
SELECT p.id   AS "id!: ProjectId",
       p.slug AS "slug!",
       COALESCE(p.owner_workspace_id = $2, false) AS "scope_row!",
       ARRAY(SELECT k.prefix FROM item_kind k
              WHERE k.project_id = p.id
              ORDER BY k.position, k.prefix COLLATE "C") AS "prefixes!"
  FROM project p
 WHERE p.id = ANY($1) OR p.owner_workspace_id = $2
```

`$2` is `NULL` when the caller does not widen, so no row matches the `OR` and `scope_row` is
`false` everywhere. `ScopeKeys { scope_row: Option<ProjectId>, projects: Vec<(ProjectId, String,
Vec<String>)> }` (`crates/htui-store/src/backend.rs` or a sibling module, re-exported). Row order
does not matter: `waiting_reads` rebuilds the qualifier in the widened scope's order.

`Backend::waiting_keys` (same signature): `Online` → `pg.waiting_keys`; `Memory` and `Offline` →
today's loop (`scope_project` when widening, then `project` + `item_kinds` per id), moved here from
`store_worker.rs`.

`waiting_reads` becomes: one `waiting_keys(widen.then_some(scope.workspace_id), &scope.project_ids)`,
`read = scope_row.map_or(scope.clone(), |row| scope.with_scope_row(row))`, then the qualifier over
`read.project_ids` in order, skipping an id the answer does not hold (today's "a project the store no
longer holds prints bare"). The local `KindsRead` trait becomes `WaitingKeysRead` with the one
method, implemented for `Backend` and `PgStore` (the CLI's `answer_online` passes a `PgStore` and is
unchanged).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming / data access | `crates/htui-store/src/pg/read.rs:3107` `item_kind_rows` | `sqlx::query_as!`/`query!` with typed aliases, `.map_err(map_sqlx)`, kind order `position, prefix COLLATE "C"` |
| Set reads | `pg/read.rs:1430` `waiting_candidates`, `pg/relay.rs:713` | `= ANY($1)` over a `Vec<Uuid>` |
| Backend dispatch | `crates/htui-store/src/backend.rs:429` `Backend::item_kinds` | `match self { Memory, Online { pg, .. }, Offline { cache, .. } }` |
| Errors | `store_worker.rs:2290` `permissions_or_unknown` | `Unreachable` propagates so `go_offline` fires; no new error mapping |
| Tests (Postgres) | `crates/htui-store/tests/cache.rs:1425` `the_mirror_lists_item_kinds_like_postgres` | Seed Postgres, compare arms' answers |
| Tests (end to end) | `crates/htui/tests/state_cli_pg.rs:404`, `store_worker.rs:4458` | Existing pins on the qualified output; must stay green unchanged |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-store/src/pg/read.rs` | UPDATE | `PgStore::waiting_keys`, the one query |
| `crates/htui-store/src/backend.rs` | UPDATE | `ScopeKeys`, `Backend::waiting_keys` (Online → one query; Memory/Offline → per-project loop) |
| `crates/htui-store/src/lib.rs` | UPDATE (if needed) | re-export `ScopeKeys` |
| `crates/htui-store/.sqlx/query-<hash>.json` | CREATE | offline entry for the new query (`cargo sqlx prepare` on a migrated scratch DB) |
| `crates/htui/src/store_worker.rs` | UPDATE | `waiting_reads` over `waiting_keys`; `KindsRead` → `WaitingKeysRead` |
| `crates/htui-store/tests/cache.rs` | UPDATE | parity test: Online (Postgres) and Offline (mirror) arms answer alike |

Not touched: `crates/htui/src/state_cmd.rs` (calls `waiting_reads(pg, …)`; the signature stays),
`htui-core` (no trait change, so no conformance `CASES` move), migrations.

## Tasks

Serial. Task 2 needs Task 1's method; there is no parallel fan-out.

### Task 1: `waiting_keys` in the store (TDD)
- **Test first** (`crates/htui-store/tests/cache.rs`): `the_mirror_answers_waiting_keys_like_postgres`.
  Seed a workspace with its scope row (`WS`), two members sharing a prefix (`FEAT`), a member with no
  kinds, and ask for an id the store does not hold. Assert on the Online answer: the scope row is
  flagged; the kindless member answers an empty prefix list; the unknown id is absent; with
  `workspace: None` no row is flagged and the scope row is absent unless asked by id. Then refresh the
  mirror and assert `Backend::Offline` answers the same `ScopeKeys` (normalised by id).
- **Action**: `PgStore::waiting_keys`, `ScopeKeys`, `Backend::waiting_keys`; `cargo sqlx prepare`.
- **Validate**: `cargo test -p htui-store --features test-support --test cache waiting_keys -- --test-threads=1`

### Task 2: `waiting_reads` over the one read
- **Action**: rewrite `waiting_reads` and the trait as in "The read"; update their doc comments
  (MOD-147 D10, D11 references stay; add MOD-170).
- **Validate**: `cargo test -p htui --lib store_worker -- --test-threads=1` (Memory arm, scope-row
  waiting tests) and `cargo test -p htui --features testkit --test state_cli_pg -- --test-threads=1`
  (Postgres CLI path: `waiting_lists_the_workspace_items_first_and_qualifies_shared_prefixes`).

## Validation

```bash
cargo test -p htui-store --features test-support --test cache -- --test-threads=1
cargo test -p htui --lib store_worker -- --test-threads=1
cargo test -p htui --features testkit --test state_cli_pg -- --test-threads=1
cargo clippy --workspace -- -D warnings          # featureless (memory: featureless clippy gate)
bash scripts/gate.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The query and the per-project loop drift (a kind order or a slug rule changes in one) | Low | Task 1's parity test runs both arms; prefixes land in a `BTreeSet`, so order cannot matter |
| `COALESCE(owner_workspace_id = $2, false)` with `$2 NULL` picks up no scope row but `OR p.owner_workspace_id = $2` mis-matches | Low | Test with `workspace: None` |
| No `.sqlx` entry → offline build fails | Medium | `cargo sqlx prepare` against a migrated scratch DB (`docs/hr-sandbox.md`); the gate builds offline |
| Session tooling: the `gortex-enforce` hook blocks read-like shell commands (`cat`, `grep`) | Medium | Read source through Gortex; build, test, prepare and git commands run |
| Sandbox Postgres holds the old `0028` checksum after the MOD-139 renumber (`now.md`) | Medium | Recreate the test DB before running the Postgres tests if migrations refuse |

## Acceptance
- [ ] Online, one tick's qualifier costs one Postgres query (was 2N+1)
- [ ] Waiting list and `htui waiting`/`status` output unchanged (existing pins green)
- [ ] Parity test green; featureless clippy and the gate green
- [ ] HANDOFF / DECISIONS close-out per `workflow-docs.md`

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `waiting_reads` reads `scope_project`, then `project` + `kinds_of` per id, serially | ✓ | `store_worker.rs:2348` body |
| Its callers are `serve_waiting` (TUI) and `state_cmd::answer_online` (CLI, `PgStore`) | ✓ | Gortex callers of `waiting_reads` |
| `Backend::Online` reads go to Postgres (`pg`) | ✓ | `backend.rs:429` `item_kinds`, `:1127` `scope_project` |
| The scope row is `project.owner_workspace_id = workspace` (unique) | ✓ | `pg/write.rs:543` `scope_row_id`; migration `0026_workspace_scope.sql:54` |
| `with_scope_row` puts the row first, once | ✓ | `htui-core/src/model/scope.rs:47` |
| Production SQL uses compile-time `query!` macros with `.sqlx` offline entries | ✓ | `pg/read.rs:3107`; `crates/htui-store/.sqlx/*.json` |
| `KeyQualifier` stores prefixes as a `BTreeSet`, so their order is irrelevant | ✓ | `htui-core/src/model/key.rs:30` |
| No `htui-core` trait changes, so no conformance `CASES` move | ✓ | plan touches only `PgStore`/`Backend` inherent methods and the `htui`-local trait |
| A Postgres pin of the qualified CLI output exists | ✓ | `crates/htui/tests/state_cli_pg.rs:404` |
| A mirror-vs-Postgres test pattern exists in `tests/cache.rs` | ✓ | `cache.rs:1425` `the_mirror_lists_item_kinds_like_postgres` |
| Task independence | n/a | Two serial tasks; no parallel marking |
