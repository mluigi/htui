# Plan: MOD-150 M1 — Link kinds in the store

**Source PRD**: `.claude/prds/mod-150-link-kinds.prd.md`
**Selected Milestone**: 1 — Link kinds in the store
**Complexity**: Medium
**Routing**: PRD path, ultracode accepted for implement and review (`/handoff-run`, 2026-11-12).

## Summary

Adds `parent`, `duplicates` and `implements` to `LinkKind` and to `item_link`'s CHECK. Postgres and
MemStore refuse, in the same order with the same sentences: a second live `parent`, a second live
`duplicates`, a `parent` that would close a cycle, and one that would make a chain deeper than 8 levels.
Partial unique indexes back the single-parent and single-canonical rules. `parent` joins the upstream
prompt walk on all three backends (Pg, MemStore, SQLite cache) and stays out of readiness. MCP `item_link`
advertises the new kinds (through `LinkKind::ALL`) and documents them, and the Graph pane labels them.

## Decisions

- **D1 — Graph words.** `parent` → "part of", `duplicates` → "duplicates", `implements` → "implements".
  Each reads `from <word> to`, like today's four (MOD-115 M4 D-1, `ui/kit/label.rs:283`). Two departures
  from ANA-35 §7.6's legend words:
  - **"duplicates", not "duplicate of":** the kind column is padded to the widest word (`graph.rs:492`
    `kind_width`), and "duplicate of" is 12 cells against "supersedes"' 10. It would widen every Graph
    snapshot.
  - **The four existing words stay.** ANA-35's "blocks" for `blocked_by` reads the edge backwards.
    "from", "relates to" and "replaces" can be adopted later with the R-TUI-5 legend toggle (MOD-120).
- **D2 — Depth.** A parent chain has at most 8 items (`MAX_PARENT_DEPTH = 8`). Proposing `from parent to`
  is refused when *ancestors of `to`, `to` included* + *the height of `from`'s subtree, `from` included*
  is more than 8. Attaching a subtree moves every descendant down, so the subtree has to be counted.
- **D3 — Concurrency (PRD open question 1).** On Pg, a `parent` proposal first takes
  `pg_advisory_xact_lock(hashtextextended('item_link:parent', 0))`. That is one database-wide key: parent
  writes are rare and cycles cross projects. It mirrors `claim_command`'s lock (`pg/write.rs:9936`). The
  cycle and depth walks run after the lock. MemStore is serialised by its state lock already.
  A `duplicates` proposal takes a per-item lock at the same point,
  `hashtextextended('item_link:duplicates:<from>', 0)`. The partial unique index alone is not
  enough. The upsert's `ON CONFLICT (from, to, kind)` absorbs only a primary-key conflict. So two
  identical first-time proposals racing past the second-canonical check make the loser trip
  `uq_item_link_one_canonical` (23505). That turns D4's idempotent upsert into a refusal. Found by
  the M1 review and fixed in round R1.
- **D4 — Refusal order.** Today's order stays: `self_link`, step/fence, own item, `to` not found, outside
  reach. Then, for `parent`: second parent, cycle, depth; for `duplicates`: second canonical.
  - **A live row with the same `to` is an idempotent upsert, never a "second".**
  - **A tombstone revival runs every check** (PRD open question 4): the PK `(from, to, kind)` revives a
    withdrawn row, so reviving a parent while another is live is refused like an insert.
  - **New sentences live in `store/traits.rs`** beside `link_outside_reach`: `second_parent(from,
    current)`, `second_canonical(from, current)`, `parent_cycle(from, to)`, `parent_too_deep(depth)`.
- **D5 — Backstop.** `uq_item_link_one_parent` and `uq_item_link_one_canonical` (partial on `deleted_at IS
  NULL`) catch any writer that skips the code check (the importer, later). Their violation surfaces as
  `map_sqlx`'s generic constraint error. No mapping is added for it.
- **D6 — Upstream walk (PRD open question 2).** `parent` joins `blocked_by` and `origin` on all three
  backends. `MAX_UPSTREAM_HOPS` stays 2, so a child sees its parent and grandparent. An item reached both
  as parent and as blocker appears once, at its minimum depth: the existing dedup (Pg `best`, Mem `seen`,
  cache `MIN`). MemStore reads a new `LinkKind::walks_upstream` const fn; the SQL keeps literal lists
  (both `.sqlx` entries regenerate).
- **D7 — Fixtures unchanged.** No `parent`/`duplicates`/`implements` row goes into
  `fixtures.rs::links`, so no prompt digest or Backlog/Graph snapshot moves. Tests plant their own rows.
- **D8 — Migration `0030_link_kinds.sql`.** Drops `item_link_kind_check`, the name verified on a scratch
  database built from `0001_init.sql`. Adds `chk_item_link_kind` with seven kinds and the two partial
  unique indexes. No cache migration: the mirror's `item_link` has no CHECK
  (`cache_migrations/0001_mirror.sql:86`), and no column changes. No new table, so `TABLES` stays.
- **D9 — MCP.** The `kind` enum grows by itself (`tools/link.rs:65`, `LinkKind::ALL`). The field doc
  lists all seven with one-line meanings ("`parent`: this item is a part of `to`"; "`duplicates`: this
  item is the same as `to`"; "`implements`: this item carries out `to`'s decision"). The new store
  refusals reach the agent through `store_error` unchanged. They are live, not pending (PRD decision:
  MOD-155 adds pending).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui-core/src/store/traits.rs:3836-3842` | refusal sentence fns `self_link`, `link_outside_reach`, `#[must_use] pub fn … -> String`, one doc line naming the decision |
| Errors | `crates/htui-store/src/pg/write.rs:9648` `propose_link` | `StoreError::Constraint(<sentence>)` before the write, inside the one transaction |
| Data access | `crates/htui-store/src/pg/write.rs:9936` | `SELECT pg_advisory_xact_lock(hashtextextended($1, 0))` for a cross-row invariant |
| Mem parity | `crates/htui-core/src/store/mem.rs:8775` `State::propose_link` | same refusal order, same sentences, plain Vec scans |
| Tests (store) | `crates/htui-core/src/store/conformance.rs:21149` `propose_link_upserts_revives_and_keeps_a_live_proposer` | conformance case named for the behaviour, `CASE` const, `live_link` helper, registered in `CASES` |
| Tests (migration) | `crates/htui-store/tests/migrations.rs` | `run_to(29)` → plant → `run_to(30)` → assert; CHECK and index names asserted from the catalog |
| Tests (concurrency) | `crates/htui-store/tests/pg_criteria.rs:6626` `two_concurrent_cancel_requests_insert_one` | two tasks race on Pg; exactly one outcome per invariant |
| Enum pin | `crates/htui-core/src/model/mod.rs:316` `link_kind_matches_check_list` | `check_enum(ALL, &[…])` in CHECK order |
| Labels | `crates/htui/src/ui/kit/label.rs:283`, test `:779` | exhaustive `match`, words table test |
| MCP tests | `crates/htui-mcp/tests/tools_backlog.rs:628` | `fx.live_link(target, kind)` after a tool call |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-core/src/model/link.rs` | UPDATE | three variants; `walks_upstream`; doc on `ProposeLink`/`UpstreamEntry` | T1 |
| `crates/htui-core/src/model/mod.rs` | UPDATE | CHECK-list pin | T1 |
| `crates/htui-core/src/store/traits.rs` | UPDATE | `MAX_PARENT_DEPTH`, four sentence fns, `propose_link` error doc | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE | `State::propose_link` checks; `State::upstream` via `walks_upstream` | T1 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | new cases + `CASES` | T1 |
| `crates/htui-store/migrations/0030_link_kinds.sql` | CREATE | CHECK swap, two partial uniques | T1 |
| `crates/htui-store/src/pg/write.rs` | UPDATE | `propose_link`: lock, single-parent/canonical, cycle, depth | T1 |
| `crates/htui-store/src/pg/read.rs` | UPDATE | `upstream_summaries` kind list | T1 |
| `crates/htui-store/src/cache/read.rs` | UPDATE | `upstream_summaries` kind list | T1 |
| `crates/htui-store/.sqlx/*.json` | CREATE/DELETE | changed and new queries (`cargo sqlx prepare`) | T1 |
| `crates/htui-store/tests/migrations.rs` | UPDATE | `0030` test; `Pending(29)` → `Pending(30)` ×4 (`:4167, :4268, :4277, :4300`) | T1 |
| `crates/htui-store/tests/connect.rs` | UPDATE | `Pending(29)` → `Pending(30)` (`:140, :242`) | T1 |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE | concurrent opposite-parent race | T1 |
| `crates/htui-store/tests/cache.rs` | UPDATE | cache upstream follows `parent` | T1 |
| `crates/htui-mcp/src/tools/link.rs` | UPDATE | `kind` field doc; module doc | T2 |
| `crates/htui-mcp/tests/tools_backlog.rs` | UPDATE | new kinds go live; a refusal reads as one line | T2 |
| `crates/htui-mcp` snapshots of `tools/list` (if any) | UPDATE | enum and doc grow; found by full insta run | T2 |
| `crates/htui/src/ui/kit/label.rs` | UPDATE | three words + words-table test | T3 |
| `crates/htui/src/ui/tabs/backlog/detail/graph.rs` | UPDATE | test: a `parent` edge reads "part of"; `kind_width` doc names the new words | T3 |

## Tasks

T1 first. T2 and T3 then run in parallel: their file sets are disjoint from each other and from T1's,
and they depend on T1 only through the `LinkKind` variants and the sentence fns.

### Task 1: Kinds, rules and the upstream walk in the store
- **Action** (tests first):
  - **Conformance cases on both stores:**
    - `parent_is_single_valued`: a second live parent is refused; the same parent again upserts; a
      withdrawn parent revived while another is live is refused.
    - `duplicates_is_single_valued`: as above, for `duplicates`.
    - `parent_refuses_a_cycle`: direct (`A→B`, then `B→A`) and three-long.
    - `parent_depth_is_at_most_eight`: a chain of 8 is accepted; the 9th is refused; attaching a
      2-high subtree under a 7-deep chain is refused.
    - `implements_links_like_relates`.
    - `upstream_follows_parent`: the child sees its parent and grandparent, the grandparent's
      grandparent is beyond 2 hops, and an item that is both parent and blocker appears once.
    - `new_kinds_never_gate_readiness`: `ready_items` and `queue_rows` are identical with and without
      the new links.
  - **Migration test `0030`:** plant links of the four old kinds, migrate, then check that the rows
    survive, the old CHECK is gone, `chk_item_link_kind` lists seven kinds, both partial indexes
    exist, and a raw second-parent INSERT fails.
  - **`pg_criteria` race:** two tasks propose `A parent B` and `B parent A` concurrently; exactly one
    succeeds and the other gets `parent_cycle`.
  - **`cache.rs`:** the cache's upstream follows `parent`.
  - **Then implement D2–D8.** Run `cargo sqlx prepare` against a migrated scratch database
    (`docs/hr-sandbox.md`).
- **Mirror**: Patterns rows Naming, Errors, Data access, Mem parity, Tests (store/migration/concurrency),
  Enum pin.
- **Validate**: `cargo test -p htui-core --all-features conformance -- --test-threads=1`; `cargo test -p htui-store --all-features --test migrations --test connect --test pg_criteria --test cache -- --test-threads=1`; `cargo sqlx prepare --check --workspace`.

### Task 2: MCP `item_link` speaks the new kinds
- **Action** (tests first):
  - **`tools_backlog.rs` tests:**
    - `item_link add` with `parent`, `duplicates` and `implements` makes each live.
    - A second parent comes back as one line naming the current parent.
    - `tools/list` lists seven kinds.
  - **Then the D9 docs.**
- **Mirror**: MCP tests row.
- **Validate**: `cargo test -p htui-mcp --all-features -- --test-threads=1`; `cargo insta test -p htui-mcp --all-features` (review every moved snapshot).

### Task 3: Graph words for the new kinds
- **Action** (tests first):
  - **Extend the words-table test** to seven pairs (D1).
  - **Add a graph unit test:** a `parent` edge renders "part of" in the kind column, and the column
    width is unchanged (still 10).
  - **Then the `label::link_kind` arms.**
- **Mirror**: Labels row.
- **Validate**: `cargo test -p htui --all-features label graph`; `cargo insta test -p htui --all-features` (expect no moved snapshot, D1/D7).

## Validation

```bash
bash scripts/gate.sh                                   # full gate (fmt, clippy --all-features, tests)
cargo clippy --workspace -- -D warnings                # featureless clippy (memory: featureless gate)
cargo test -p htui-core --all-features conformance -- --test-threads=1
cargo test -p htui-store --all-features -- --test-threads=1   # Postgres conformance on the sandbox PG
cargo sqlx prepare --check --workspace
cargo insta test --workspace --all-features            # full insta run; review every moved snapshot
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `item_link`'s kind CHECK is auto-named `item_link_kind_check` | TRUE | scratch DB from `0001_init.sql`: `item_link_kind_check CHECK (kind = ANY (ARRAY['blocked_by','origin','relates','supersedes']))`; no later migration touches `item_link` (text search over `migrations/`) |
| PK is `(from_item_id, to_item_id, kind)` and propose upserts/revives on it | TRUE | scratch DB `item_link_pkey`; `pg/write.rs:9648` `ON CONFLICT (from_item_id, to_item_id, kind) DO UPDATE … deleted_at = NULL` |
| Latest Postgres migration is `0029`; next is `0030` | TRUE | file listing `crates/htui-store/migrations/0029_divided_fanout.sql` |
| The cache mirror's `item_link` has no CHECK, so no cache migration is needed | TRUE | `cache_migrations/0001_mirror.sql:86`; ANA-35 §2.7 "the mirror has no CHECKs or FKs" |
| Readiness reads only `blocked_by` on every backend | TRUE | `pg/read.rs:156, 2308, 2593` and `cache/read.rs:251` filter `kind = 'blocked_by'`; `mem.rs:1728` `is_ready`, `:6145` `open_blocker_keys` match `LinkKind::BlockedBy` |
| The upstream walk lists `blocked_by`, `origin` in three places | TRUE | `pg/read.rs:625, 630`; `cache/read.rs:923, 927`; `mem.rs:1908` |
| `MAX_UPSTREAM_HOPS` is 2 | TRUE | `store/traits.rs:101` |
| The `links()` traversal follows every kind (the Graph shows new kinds with no read change) | TRUE | no `kind` filter in the `'blocked_by'` search hits for `PgStore.links` (`pg/read.rs:224`) / `CacheStore.links`; the Mem walk matches no kind (`LinkKind::` hits list none in `MemStore.links`) |
| No orchestrator production code matches on `LinkKind` | TRUE | `LinkKind::` text search: production hits only in `fixtures.rs`, `mem.rs`, `mcp/tools/link.rs`, `ui/kit/label.rs`, `graph.rs` |
| Exhaustive `match` on `LinkKind` exists only in `label::link_kind` | TRUE | same search; other sites use `ALL`, `==` or `matches!` |
| MCP's kind enum derives from `LinkKind::ALL` | TRUE | `mcp/tools/link.rs:65` `kind_schema` |
| An advisory-lock precedent exists | TRUE | `pg/write.rs:9936` `pg_advisory_xact_lock(hashtextextended($1, 0))` in `claim_command` |
| "duplicate of" would widen the Graph kind column | TRUE | `graph.rs:492` `kind_width` = max over `ALL`; "supersedes" = 10, "duplicate of" = 12, "duplicates" = 10, "part of" = 7, "implements" = 10 |
| Migration-count pins are `Pending(29)` in six places | TRUE | `tests/connect.rs:140, 242`; `tests/migrations.rs:4167, 4268, 4277, 4300` (both `MigrationState` and `HeadlessError`) |
| T1, T2, T3 file sets are pairwise disjoint | TRUE | Files table: T1 ⊂ `htui-core`/`htui-store`; T2 ⊂ `htui-mcp`; T3 ⊂ `crates/htui/src/ui`; intersection ∅ |
| Fixtures carry no new-kind links, so no digest moves | TRUE by D7 | `fixtures.rs:1325` `links()` lists only the four old kinds; D7 adds none |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `.sqlx` drift from the changed upstream query and the new propose queries | High | `cargo sqlx prepare` on a migrated scratch DB; `--check` in validation |
| Migration number collides with a parallel branch at collect | Medium | renumber at `scripts/hr collect`; repin the six `Pending(N)` sites |
| The advisory lock serialises unrelated parent writes | Low | parent writes are rare; scope the lock to `kind = parent` only |
| Depth walk cost on a large subtree | Low | bounded: refuse once the count passes 8; recursive CTE with a depth guard |
| `tools/list` snapshot moves | Medium | T2 reviews it; expected and accepted |
| Sandbox gate flakes (live Qdrant, scheduling) | High | `--test-threads=1` for store suites; live-Qdrant owed on the host (memory) |

## Acceptance

- [ ] All tasks complete, tests written before code
- [ ] Validation passes (sandbox), host Postgres/Qdrant gate noted as owed
- [ ] Patterns mirrored, not reinvented
- [ ] PRD M1 row → complete; HANDOFF phase note (P1)
