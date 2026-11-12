# Blueprint: MOD-150 M1 — Link kinds in the store

**Plan**: `.claude/plans/mod-150-m1-link-kinds.plan.md` (D1–D9 binding, not reopened here).
**PRD**: `.claude/prds/mod-150-link-kinds.prd.md`. **Design**: `docs/ANA-35.md` §7.6, §7.9 (P7).
**Tasks**: T1 (store) first; then T2 (MCP) and T3 (TUI) in parallel. File sets are disjoint.

Read "Plan deviations" (at the end) before you start: it adds pin sites and test files the plan's
Files table left out, all of them inside T1's crates.

---

## T1 — Kinds, rules and the upstream walk in the store

### T1.1 `crates/htui-core/src/model/link.rs`

Add three variants **after** `Supersedes`, inside `str_enum!` (this order is `LinkKind::ALL`'s, and
it is the CHECK's order):

```rust
        /// `from` is a part of `to`, its parent (MOD-150, ANA-35 §7.6): at most one live parent
        /// per `from`, no cycles, a chain at most `MAX_PARENT_DEPTH` items deep. Walks upstream,
        /// never gates readiness.
        Parent => "parent",
        /// `from` is the same as `to`, its canonical item: at most one live canonical per `from`.
        Duplicates => "duplicates",
        /// `from` carries out `to`'s decision or specification.
        Implements => "implements",
```

After the macro, add:

```rust
impl LinkKind {
    /// MOD-150 D6: whether the upstream prompt walk
    /// ([`ReadStore::upstream_summaries`](crate::store::ReadStore::upstream_summaries)) follows
    /// this kind from `from` to `to`: `blocked_by`, `origin` and `parent`. The SQL backends spell
    /// the same list literally (`pg/read.rs`, `cache/read.rs`).
    #[must_use]
    pub const fn walks_upstream(self) -> bool {
        matches!(self, Self::BlockedBy | Self::Origin | Self::Parent)
    }
}
```

Doc edits:
- `ProposeLink.to`: replace "an item of the same project" with "an item within the run's reach
  (MOD-147 D3)". Add to `ProposeLink.kind`: "`parent` and `duplicates` are single-valued, and a
  `parent` is checked for cycles and depth (MOD-150 D4)."
- `UpstreamEntry` doc: "over `blocked_by` and `origin` edges" becomes "over `blocked_by`, `origin`
  and `parent` edges (MOD-150 D6)".

Unit test in the file's `mod tests`:
`walks_upstream_is_blocked_by_origin_and_parent`: the `LinkKind::ALL` entries that walk upstream are
exactly `[BlockedBy, Origin, Parent]`, in that order.

### T1.2 `crates/htui-core/src/model/mod.rs` (`:316`)

```rust
    fn link_kind_matches_check_list() {
        check_enum(
            LinkKind::ALL,
            &["blocked_by", "origin", "relates", "supersedes", "parent", "duplicates", "implements"],
        );
    }
```

### T1.3 `crates/htui-core/src/store/traits.rs`

Below `MAX_UPSTREAM_HOPS` (`:101`):

```rust
/// MOD-150 D2 (ANA-35 §7.6): a parent chain holds at most this many items, the bottom child and
/// the top parent included. `from parent to` is refused when the ancestors of `to` (`to`
/// included) plus the height of `from`'s subtree (`from` included) exceed it: attaching a subtree
/// moves every descendant down. Both stores walk at most `MAX_PARENT_DEPTH + 1` levels each way,
/// so the depth the refusal names is the same on both.
pub const MAX_PARENT_DEPTH: u8 = 8;
```

After `link_outside_reach` (`:3840`), the four sentences (D4; one doc line naming the decision, as
`self_link` and `link_outside_reach` do):

```rust
/// MOD-150 D4 (ANA-35 §7.6): an item has at most one live `parent`; `current` is the live one.
#[must_use]
pub fn second_parent(from: ItemId, current: ItemId) -> String {
    format!("item {from} already has a parent ({current}); an item has at most one")
}

/// MOD-150 D4 (ANA-35 §7.6): an item duplicates at most one live canonical item, `current`.
#[must_use]
pub fn second_canonical(from: ItemId, current: ItemId) -> String {
    format!("item {from} already duplicates {current}; an item has at most one canonical item")
}

/// MOD-150 D4 (ANA-35 §7.6): `from parent to` would close a cycle, `from` being above `to`.
#[must_use]
pub fn parent_cycle(from: ItemId, to: ItemId) -> String {
    format!("item {to} is already part of {from}; a parent link would close a cycle")
}

/// MOD-150 D2: `from parent to` would make a parent chain `depth` items deep.
#[must_use]
pub fn parent_too_deep(depth: usize) -> String {
    format!("a parent chain would be {depth} items deep; at most {MAX_PARENT_DEPTH} are allowed")
}
```

`WriteStore::propose_link` doc (`:2437-2446`). Keep the first paragraph and add a sentence: "MOD-150 D3:
on Postgres a `parent` proposal takes one database-wide advisory lock before its walks." Replace
`# Errors` with:

```rust
    /// # Errors
    /// `Constraint(self_link(..))` when `from == to` (before any read); `NotFound { run_step }`;
    /// `Fenced`; `Constraint(step_writes_own_item(..))` when `from` is not the run's item;
    /// `NotFound { entity: "item" }` for `to`; `Constraint(link_outside_reach(..))` when `to` is
    /// outside the run's reach (MOD-147 D3): not in the run's project, in no workspace holding it
    /// (its scope row) and in none of those workspaces' member projects. Then (MOD-150 D4), for
    /// `parent`: `Constraint(second_parent(..))` when `from` has a live parent other than `to`,
    /// `Constraint(parent_cycle(..))` when `from` is among `to`'s ancestors, and
    /// `Constraint(parent_too_deep(..))` when the chain would pass [`MAX_PARENT_DEPTH`]; for
    /// `duplicates`: `Constraint(second_canonical(..))` when `from` has a live canonical other
    /// than `to`. A live row with the same `to` is an upsert, never a second one; reviving a
    /// tombstone runs every check, as an insert does.
```

`ReadStore::upstream_summaries` doc (`:144`, `:152`): "over `blocked_by` and `origin` edges" becomes
"over `blocked_by`, `origin` and `parent` edges"; "and only those two link kinds" becomes "and only
the kinds [`LinkKind::walks_upstream`](crate::model::LinkKind::walks_upstream) names (MOD-150 D6)".

`crates/htui-core/src/store/mod.rs`: add `MAX_PARENT_DEPTH`, `parent_cycle`, `parent_too_deep`,
`second_canonical`, `second_parent` to the `pub use traits::{…}` list, in its alphabetical order
(`MAX_UPSTREAM_HOPS` is re-exported there too). This file is not in the plan's Files table; see
deviation 8.

### T1.4 `crates/htui-core/src/store/mem.rs`

Import the four fns and `MAX_PARENT_DEPTH` beside `link_outside_reach` (`:81`).

**`State::upstream` (`:1908`)**: replace
`&& matches!(link.kind, LinkKind::BlockedBy | LinkKind::Origin)` with `&& link.kind.walks_upstream()`.
The `seen` dedup already keeps an item reached as both parent and blocker once, at its minimum
depth (BFS by level).

**New helpers on `impl State`**, beside `link_reaches`:

```rust
    /// MOD-150 D4: the `to` of `from`'s live `kind` link other than `to`, if any.
    fn live_other(&self, from: ItemId, kind: LinkKind, to: ItemId) -> Option<ItemId> {
        self.links
            .iter()
            .find(|row| {
                row.deleted_at.is_none()
                    && row.kind == kind
                    && row.from_item_id == from
                    && row.to_item_id != to
            })
            .map(|row| row.to_item_id)
    }

    /// MOD-150 D2: walks live `parent` links level by level from `start` (level 1), upward
    /// (`from` -> `to`) or downward (`to` -> `from`), at most `MAX_PARENT_DEPTH + 1` levels.
    /// Answers the levels reached and whether `seek` was among the items reached. This mirrors
    /// PgStore's recursive CTEs row for row (a level is every item one more hop out), so both
    /// stores count the same depth even on data a backstop-skipping writer left inconsistent.
    fn parent_walk(&self, start: ItemId, upward: bool, seek: Option<ItemId>) -> (usize, bool) {
        let cap = usize::from(MAX_PARENT_DEPTH) + 1;
        let mut level = vec![start];
        let mut levels = 1;
        let mut found = seek == Some(start);
        while levels < cap {
            let next: Vec<ItemId> = self
                .links
                .iter()
                .filter(|row| row.deleted_at.is_none() && row.kind == LinkKind::Parent)
                .filter_map(|row| {
                    let (near, far) = if upward {
                        (row.from_item_id, row.to_item_id)
                    } else {
                        (row.to_item_id, row.from_item_id)
                    };
                    level.contains(&near).then_some(far)
                })
                .collect();
            if next.is_empty() {
                break;
            }
            found |= seek.is_some_and(|item| next.contains(&item));
            level = next;
            levels += 1;
        }
        (levels, found)
    }
```

**`State::propose_link` (`:8775`)**: insert between the `link_reaches` refusal and
`let at = self.links.iter().position(…)`, so these run before the upsert and therefore also before a
tombstone's revival (D4):

```rust
        // MOD-150 D4: one live parent, one live canonical. A live row with the same `to` is the
        // upsert below, never a second one; a tombstone's revival is checked like an insert.
        if matches!(link.kind, LinkKind::Parent | LinkKind::Duplicates) {
            if let Some(current) = self.live_other(link.from, link.kind, link.to) {
                return Err(StoreError::Constraint(if link.kind == LinkKind::Parent {
                    second_parent(link.from, current)
                } else {
                    second_canonical(link.from, current)
                }));
            }
        }
        if link.kind == LinkKind::Parent {
            let (above, cycle) = self.parent_walk(link.to, true, Some(link.from));
            if cycle {
                return Err(StoreError::Constraint(parent_cycle(link.from, link.to)));
            }
            let (below, _) = self.parent_walk(link.from, false, None);
            let depth = above + below;
            if depth > usize::from(MAX_PARENT_DEPTH) {
                return Err(StoreError::Constraint(parent_too_deep(depth)));
            }
        }
```

Extend the method's doc comment with one sentence naming the MOD-150 checks and their order. The
MemStore state lock already serialises them (D3).

### T1.5 `crates/htui-store/migrations/0030_link_kinds.sql` (CREATE), full text

```sql
-- 0030_link_kinds.sql - MOD-150 milestone 1 (plan D5, D8; ANA-35 §7.6, §7.9 P7): item_link takes
-- three more kinds. Forward-only (R-STO-5). Depends on 0001_init.sql (item_link).
--
-- parent (child -> parent, "is a part of"), duplicates (duplicate -> canonical, "is the same as")
-- and implements (item -> item, "carries out that decision"). 0001 wrote the kind CHECK inline and
-- unnamed, so Postgres named it item_link_kind_check (verified on a scratch database built from
-- 0001_init.sql). It is replaced by the named chk_item_link_kind: the four old kinds first, in
-- their order, then the three new ones, which is LinkKind::ALL's order. Every existing row passes.
--
-- A child has at most one live parent and an item at most one live canonical. The two partial
-- unique indexes back the store's own refusals (PgStore::propose_link's second_parent and
-- second_canonical) against a writer that skips them; their violation surfaces as map_sqlx's
-- generic constraint error. A tombstoned row (deleted_at set) is outside both, so a withdrawn
-- parent may sit beside a live one. No constraint expresses acyclicity or the depth of 8: the store
-- checks both, under an advisory lock (plan D3).
--
-- Not mirrored: the cache's item_link has no CHECK and no column changes, so there is no cache
-- migration. cache_meta.schema_version moves to 30 through PgStore::schema_version(), so each box
-- rebuilds its mirror once on first start; a headless worker never migrates: migrate from a TUI
-- first.

ALTER TABLE item_link DROP CONSTRAINT item_link_kind_check;
ALTER TABLE item_link ADD CONSTRAINT chk_item_link_kind CHECK (kind IN
    ('blocked_by', 'origin', 'relates', 'supersedes', 'parent', 'duplicates', 'implements'));

CREATE UNIQUE INDEX uq_item_link_one_parent ON item_link (from_item_id)
    WHERE kind = 'parent' AND deleted_at IS NULL;
CREATE UNIQUE INDEX uq_item_link_one_canonical ON item_link (from_item_id)
    WHERE kind = 'duplicates' AND deleted_at IS NULL;
```

### T1.6 `crates/htui-store/src/pg/write.rs` `PgStore::propose_link` (`:9648`)

Import `MAX_PARENT_DEPTH, parent_cycle, parent_too_deep, second_canonical, second_parent` in the
`htui_core::store::traits::{…}` list (`:63`). Rows are `item_link` with `deleted_at` tombstones and
PK `(from_item_id, to_item_id, kind)`. Insert between `if !reachable { … }` and the `INSERT … ON
CONFLICT` upsert, inside the same `tx`:

```rust
        // MOD-150 D3: parent writes are serialised database-wide (one key: parent writes are rare
        // and cycles cross projects), so the walks below see every committed parent link. Under
        // READ COMMITTED each statement after the lock reads a fresh snapshot. This is
        // `claim_command`'s statement text verbatim, so it reuses that query's `.sqlx` entry.
        if link.kind == LinkKind::Parent {
            sqlx::query!(
                "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
                "item_link:parent",
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }
        // MOD-150 D4: one live parent, one live canonical. A live row with the same `to` is the
        // upsert below; a tombstone's revival is checked like an insert. `duplicates` takes no
        // lock: uq_item_link_one_canonical backs it (D3, D5).
        if matches!(link.kind, LinkKind::Parent | LinkKind::Duplicates) {
            let current = sqlx::query_scalar!(
                r#"
                SELECT to_item_id AS "to_item_id: ItemId"
                  FROM item_link
                 WHERE from_item_id = $1 AND kind = $2 AND to_item_id <> $3
                   AND deleted_at IS NULL
                 ORDER BY to_item_id
                 LIMIT 1
                "#,
                link.from.as_uuid(),
                link.kind.as_str(),
                link.to.as_uuid(),
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if let Some(current) = current {
                return Err(StoreError::Constraint(if link.kind == LinkKind::Parent {
                    second_parent(link.from, current)
                } else {
                    second_canonical(link.from, current)
                }));
            }
        }
        if link.kind == LinkKind::Parent {
            // Each walk stops at MAX_PARENT_DEPTH + 1 levels, as MemStore's does: the guard ends a
            // cycle another writer left behind, and both stores report the same depth.
            let levels = i32::from(MAX_PARENT_DEPTH) + 1;
            // `to` and its ancestors (level 1 is `to`); `cycle` when `from` is among them.
            let above = sqlx::query!(
                r#"
                WITH RECURSIVE up(item_id, n) AS (
                        SELECT $1::uuid, 1
                    UNION ALL
                        SELECT l.to_item_id, up.n + 1
                          FROM up
                          JOIN item_link l ON l.from_item_id = up.item_id
                         WHERE l.kind = 'parent' AND l.deleted_at IS NULL AND up.n < $3
                )
                SELECT COALESCE(bool_or(item_id = $2), FALSE) AS "cycle!",
                       MAX(n)                                AS "levels!"
                  FROM up
                "#,
                link.to.as_uuid(),
                link.from.as_uuid(),
                levels,
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if above.cycle {
                return Err(StoreError::Constraint(parent_cycle(link.from, link.to)));
            }
            // The height of `from`'s subtree (level 1 is `from`): live parent links pointing at it.
            let below = sqlx::query_scalar!(
                r#"
                WITH RECURSIVE down(item_id, n) AS (
                        SELECT $1::uuid, 1
                    UNION ALL
                        SELECT l.from_item_id, down.n + 1
                          FROM down
                          JOIN item_link l ON l.to_item_id = down.item_id
                         WHERE l.kind = 'parent' AND l.deleted_at IS NULL AND down.n < $2
                )
                SELECT MAX(n) AS "levels!" FROM down
                "#,
                link.from.as_uuid(),
                levels,
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            let depth = usize::try_from(above.levels + below).unwrap_or(usize::MAX);
            if depth > usize::from(MAX_PARENT_DEPTH) {
                return Err(StoreError::Constraint(parent_too_deep(depth)));
            }
        }
```

Notes: the anchor's `$1::uuid` fixes the CTE column type. `n` is `int4`, so `$3`/`$2` infer `int4`
(bind `i32`). Postgres admits a recursive term that references `up`/`down` exactly once (true
here). Early `return Err` drops `tx`, which rolls back and releases the lock. The MemStore level
counting in T1.4 equals `MAX(n)` here, and `found` equals `bool_or`.

### T1.7 Upstream kind lists

- `crates/htui-store/src/pg/read.rs` `upstream_summaries` (`:625`, `:630`): both
  `AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL` become
  `AND l.kind IN ('blocked_by','origin','parent') AND l.deleted_at IS NULL`. Change nothing else in
  the text: `best`'s `MIN(depth)` is the dedup (D6). Update the method doc if it names the two kinds.
- `crates/htui-store/src/cache/read.rs` `upstream_summaries` (`:923`, `:927`): both
  `kind IN ('blocked_by','origin')` become `kind IN ('blocked_by','origin','parent')`. Both sites
  sit inside the `format!` string. The mirror holds only live rows, so no `deleted_at` filter applies.
- Mem: T1.4.

### T1.8 `.sqlx` regeneration (sandbox Postgres `localhost:5439`, user `postgres`, trust auth;
`docs/hr-sandbox.md` §"Changing SQL queries in a run")

```bash
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;" \
  -c "CREATE DATABASE htui_sqlx;"
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features   # keep both flags or test-only queries vanish
cargo sqlx prepare --check                           # "potentially unused queries" warning is expected
```

Expected `.sqlx` delta: one file replaced (the Pg upstream query, `query-5d8f75bb…` deleted and a
new hash added) and three new files (the second-live query and the two CTEs). The advisory lock
reuses `query-751f836d…`. Stage the deleted file as well as the new ones: `git add -A
crates/htui-store/.sqlx`.

### T1.9 Tests

#### Conformance cases (`crates/htui-core/src/store/conformance.rs`)

Add a block comment `// MOD-150 M1: link kinds (plan D2-D7)` at the end of `CASES` (after
`"a_phase_naming_no_judge_agent_is_refused"`, `:392`) with the seven names below. Add seven boxed
arms at the end of `run_case_inline` (after `:1224-1226`), in the shape
`"name" => Box::pin(name(store)).await,`. Put the fns in a new section at the end of the cases,
before `#[cfg(test)] mod tests`. Import the four sentence fns and `MAX_PARENT_DEPTH` in the
`crate::store::traits::{…}` list.

Shared tools already there: `agent_step(case, store, project, item, None)` gives an **unleased**
run and step, so use `StepFence::Unleased`. No claim is made, so box capacity never interferes.
`live_link`, `constraint_exactly`, `mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, title))`
(as `mint_consecutive_keys`), `finish_run(run, RunStatus::Cancelled, None, seam_clock())` (as
`:13305`), `platform_scope()`, `PromptScope::project_only(..)`, `withdraw_link`. Add one local
helper:

```rust
/// MOD-150: a fresh `htui` FEAT item, its id.
async fn feat<S: WriteStore>(case: &str, store: &S, title: &str) -> ItemId {
    store.mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, title)).await.expect(case).id
}
```

Each case:

1. **`parent_is_single_valued`**: `x = ids::HTUI_ANA_2` with one step. (a) `x parent HTUI_FEAT_1`
   is live. (b) Proposing it again is `Ok`, with the same `created_at` and still live (an upsert).
   (c) `x parent HTUI_FEAT_3` gives `constraint_exactly(…, &second_parent(x, HTUI_FEAT_1), …)` and
   `!live_link(x, FEAT_3, Parent)`. (d) `withdraw_link` the FEAT_1 parent, then `x parent FEAT_3` is
   live. (e) Proposing `x parent FEAT_1` again would revive its tombstone; it is refused with
   `second_parent(x, HTUI_FEAT_3)` and FEAT_1 stays not live. (f) `x relates HTUI_FEAT_1` is still
   `Ok`, because the rule binds the kind only.
2. **`duplicates_is_single_valued`**: the same steps (a)–(e) for `Duplicates` with
   `second_canonical`. Finish with (f): `x parent HTUI_FEAT_1` is `Ok` beside the live duplicates
   link (kinds do not interfere).
3. **`parent_refuses_a_cycle`**: `a, b, c = feat(…)`, each with its own `agent_step`. Direct:
   `a parent b` ok; `b parent a` gives `parent_cycle(b, a)`, not live. Three-long: `b parent c` ok;
   `c parent a` gives `parent_cycle(c, a)`, not live. Control: `c relates a` is `Ok` (only `parent`
   walks).
4. **`parent_depth_is_at_most_eight`**: `i[0..9] = feat(…)` (nine items), steps for `i[1..9]`.
   For k in 1..8, `i[k] parent i[k-1]` is ok, so `i[0..8]` is a chain of eight items. Then
   `i[8] parent i[7]` gives `parent_too_deep(9)`, not live. Subtree: `j1, j2 = feat(…)` with steps;
   `j2 parent j1` ok (`j1`'s subtree is two high). `j1 parent i[6]` (7 deep at `i[6]`) gives
   `parent_too_deep(9)`, while `j1 parent i[5]` (6 + 2 = 8) is ok, the positive boundary. Assert
   `MAX_PARENT_DEPTH == 8` once, so a change to it shows in this case.
5. **`implements_links_like_relates`**: `x = HTUI_ANA_2`. `x implements HTUI_FEAT_1` and
   `x implements HTUI_FEAT_3` are both live (no single-value rule). Re-proposing upserts (same
   `created_at`, proposer kept). `x implements VULKAN_FEAT_1` gives
   `link_outside_reach(VULKAN_FEAT_1)`. `withdraw_link` tombstones one.
6. **`upstream_follows_parent`**: `c, p, g, gg = feat(…)`, steps for `c, p, g`. `c parent p`,
   `p parent g`, `g parent gg`. Let
   `up = |hops| store.upstream_summaries(c, hops, &PromptScope::project_only(ids::PROJECT_HTUI))`.
   `up(2)` maps to `(item_id, depth)` == `[(p, 1), (g, 2)]` (gg is three hops out). `up(1)` is
   `[(p, 1)]`. Then `c blocked_by g` and `c blocked_by p` (c's step): `up(2)` == `[(p, 1), (g, 1)]`,
   in canonical order (sort the expected pair with `UpstreamEntry::sort_canonical` or compare as a
   set and also check `entries.len() == 2`). Each item appears once, at its minimum depth.
7. **`new_kinds_never_gate_readiness`**: let `ready = items(&platform_scope(), &ItemFilter {
   ready: Some(true), ..Default })` and `states = items(&platform_scope(), &ItemFilter::default())`
   (the full rows, derived state included). Take a baseline of both. For each
   `(from, kind, to)` in `[(AGY_FEAT_1, Parent, HTUI_FEAT_1), (AGY_FIX_1, Duplicates, HTUI_FEAT_2),
   (HTUI_ANA_2, Implements, HTUI_FEAT_1)]`, give `from` an `agent_step`, propose, then `finish_run`
   the run as `Cancelled` (a cancelled run does not hold readiness). HTUI_FEAT_1 is running
   (unreleased) and FEAT_2 is blocked, so a `blocked_by` to them would gate. Assert `ready` and
   `states` equal the baseline (`ready` is `[HTUI_ANA_2, AGY_FEAT_1, AGY_FIX_1]`, as in
   `filter_status_project_tags_ready`). Control: a new `agent_step` on `AGY_FEAT_1`, then
   `AGY_FEAT_1 blocked_by HTUI_FEAT_1`, then cancel: `AGY_FEAT_1` leaves `ready`. That proves the
   case can see gating.

Pins that move with the case count, 277 → **284** (deviation 1):
- `crates/htui-core/tests/mem_store.rs:37`: `277` becomes `284`. Append to the message: ", and
  MOD-150 M1's seven link-kind cases (plan D2-D7)".
- `crates/htui-store/tests/pg_conformance.rs:59`: `EXPECTED_CASES` becomes `284`. Add the doc line
  "/// and MOD-150 M1's seven link-kind cases (plan D2-D7) make it 284." Update the assert message
  ("284 since …, and MOD-150 M1's seven link-kind cases").

Gotcha (`every_cross_referenced_test_name_exists`, `conformance.rs:~30522`): in `conformance.rs`
and `mem.rs` doc comments, an inline-code span holding a snake_case name with four or more `_` is
checked as a test name. Do not write `` `uq_item_link_one_parent` `` or
`` `uq_item_link_one_canonical` `` in backticks there; write them bare, or say "the partial unique
index". A reference to the race test, if you add one, must read
`` `pg_criteria.rs::two_concurrent_opposite_parents_close_no_cycle` `` exactly.

#### Migration test (`crates/htui-store/tests/migrations.rs`, after the 0029 tests)

`migration_0030_widens_link_kinds_and_backs_one_parent`, following
`migration_0026_backfills_document_project_and_keeps_item`:
1. `common::bare_db()`, `MIGRATOR.run_to(29, &db.pool)` ("apply 0001 through 0029"). Set up
   `user = insert_user(pool, "mod150")` and `pk = plant_project_kind(pool, user, "links")`. Plant
   `a, b, c, d = plant_item(pool, pk, 1..=4, "open", None, user)`.
2. Plant `a -> b` of each old kind with raw SQL:
   `INSERT INTO item_link (from_item_id, to_item_id, kind) VALUES ($1, $2, $3)` for
   `blocked_by, origin, relates, supersedes`.
3. `MIGRATOR.run(pool)` ("apply 0030").
4. The four rows survive: `SELECT kind FROM item_link WHERE from_item_id = $1 ORDER BY kind`
   gives the four kinds.
5. The old CHECK is gone:
   `SELECT count(*) FROM pg_constraint WHERE conrelid = 'item_link'::regclass AND conname = 'item_link_kind_check'`
   == 0.
6. `pg_get_constraintdef` of `chk_item_link_kind` (as `:2048`) ==
   `"CHECK ((kind = ANY (ARRAY['blocked_by'::text, 'origin'::text, 'relates'::text, 'supersedes'::text, 'parent'::text, 'duplicates'::text, 'implements'::text])))"`.
7. Both indexes exist: `SELECT indexdef FROM pg_indexes WHERE schemaname = 'public' AND indexname
   = $1` for each name equals
   `"CREATE UNIQUE INDEX uq_item_link_one_parent ON public.item_link USING btree (from_item_id) WHERE ((kind = 'parent'::text) AND (deleted_at IS NULL))"`
   and the same with `uq_item_link_one_canonical` / `'duplicates'::text`. Check the exact text on
   the first run, and keep the assert as an equality.
8. Backstop: `a parent b` inserts; `a parent c` raw is refused, `assert_refused_by(err,
   "uq_item_link_one_parent", "a second live parent")`. `UPDATE item_link SET deleted_at = now()
   WHERE from_item_id = a AND kind = 'parent'`; then `a parent c` inserts (the tombstone is outside
   the index). The same for `duplicates` (`a duplicates b`, then `a duplicates d` refused by
   `uq_item_link_one_canonical`).
9. `kind = 'blocks'` is refused by `chk_item_link_kind` (`assert_refused_by`).

Migration-count pins, 29 → **30** (deviation 2). Every one of them, with its text ("twenty-nine" →
"thirty", "through MOD-139's 0029_divided_fanout.sql" → "through MOD-150's 0030_link_kinds.sql"):
- `tests/migrations.rs:118-119`: the `applied` vec gains `30`. Append to the message: "and
  MOD-150's 0030_link_kinds.sql".
- `tests/migrations.rs:4167` `MigrationState::Pending(30)`; `:4268` `Pending(30)`; `:4277`
  `HeadlessError::MigrationsPending(30)`; `:4300` `MigrationsPending(30)`.
- `tests/migrations.rs:4452`: the `_sqlx_migrations` count `29` becomes `30`.
- `tests/connect.rs:140` `Pending(30)`; `:156` `pending, 30`; `:242` `Pending(30)`.
- `TABLES` and its `52` stay (no new table, D8).

#### Pg concurrency (`crates/htui-store/tests/pg_criteria.rs`)

`two_concurrent_opposite_parents_close_no_cycle`, `#[tokio::test(flavor = "multi_thread")]`,
shaped like `two_concurrent_cancel_requests_insert_one` (`:6613`), using
`a_step_document_racing_a_park_never_deadlocks`'s extra pools (`:7212`):
- `common::demo_db()`. Open two more pools with `PgStore::connect(&db.url, &db.identity)`
  (`left`, `right`).
- 10 rounds. Each round: `a, b = db.store.mint_item(race_item(ids::KIND_HTUI_FEAT, …))`, a
  `create_run(race_run(a))` and `create_run(race_run(b))`, and one `NewRunStep` each (position 0,
  as `:7224`).
- `tokio::join!(left.propose_link(StepFence::Unleased, a parent b by step_a), right.propose_link(StepFence::Unleased, b parent a by step_b))`.
- Exactly one is `Ok`. The other is `Err(StoreError::Constraint(s))` with `s == parent_cycle(from,
  to)` of the loser's own link; panic on anything else ("round {round}: …").
- `SELECT count(*) FROM item_link WHERE kind = 'parent' AND deleted_at IS NULL AND from_item_id = ANY($1)`
  over `[a, b]` == 1.
- Close the extra pools, `db.drop_db()`.
Without the lock both would pass the walk under READ COMMITTED and both rows would land, so this
test fails if the lock is removed.

`new_link_kinds_never_gate_ready_items_or_queue_rows` (deviation 3). It covers the inherent reads
the conformance case cannot reach, on both stores, shaped like `queue_rows_answer_alike_on_both_stores`
(`:11252`):
- `pg = &db.store`. `bare = MemStore::demo()`. `linked = MemStore::from_demo(data)`, where `data =
  fixtures::demo_data()` with three `ItemLink`s pushed (proposer `None`, live):
  `AGY_FEAT_1 parent HTUI_FEAT_1`, `AGY_FIX_1 duplicates HTUI_FEAT_2`, `HTUI_ANA_2 implements HTUI_FEAT_1`.
- Queue `AGY_FEAT_1, AGY_FIX_1, HTUI_ANA_2` on `ids::BOX` in all three stores (`queue_item(item,
  ids::BOX, None, ids::USER, at)`).
- On Pg: read `ready_items(&scope, ids::BOX)` and `queue_rows(ids::BOX)` before, raw-INSERT the
  same three rows, read again: equal. Use the `Scope` `ready_items_order_by_priority_then_created_at`
  uses.
- Mem: `bare` and `linked` answer equal `ready_items` / `queue_rows`, and `queue_rows`'
  `open_blockers` stay as `bare`'s.

#### Cache (`crates/htui-store/tests/cache.rs`)

`the_mirror_upstream_follows_parent`, shaped like `a_cycle_never_renders_the_root_as_its_own_upstream`
(`:2410`):
- `demo_db`, `open_cache(&db)`. Raw-INSERT into Pg `AGY_FIX_1 parent AGY_FEAT_1` and
  `AGY_FEAT_1 parent HTUI_FEAT_3` (`kind = 'parent'`, `created_at`/`updated_at` bound).
  `run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))`.
- `MemStore::from_demo(data)` with the same two `ItemLink`s pushed.
- `scope = PromptScope::from_scope(&platform_scope().await, ids::PROJECT_AGY)`, and
  `upstream_summaries(ids::AGY_FIX_1, 2, &scope)` on all three.
- On each backend: `AGY_FEAT_1` at depth 1 and `HTUI_FEAT_3` at depth 2. `HTUI_FEAT_2` is absent
  (`AGY_FEAT_1 relates FEAT_2` is not walked). `pg == mem` and `mirror == mem`.
- `teardown(db, &[&cache])`.

### T1 commits (explicit path staging, each ending `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`)

1. `feat(core): MOD-150 parent, duplicates and implements link kinds`: `model/link.rs`,
   `model/mod.rs`, `store/traits.rs`, `store/mod.rs`.
2. `feat(core): MOD-150 MemStore refuses a second parent or canonical, a cycle and a deep chain`:
   `store/mem.rs`, `store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`,
   `crates/htui-store/tests/pg_conformance.rs` (the pin moves with `CASES`).
3. `feat(store): MOD-150 0030_link_kinds and the Postgres link rules`: the migration, `pg/write.rs`,
   `pg/read.rs`, `cache/read.rs`, `.sqlx/` (`git add -A crates/htui-store/.sqlx`),
   `tests/migrations.rs`, `tests/connect.rs`.
4. `test(store): MOD-150 opposite-parent race, readiness and the mirror's upstream`:
   `tests/pg_criteria.rs`, `tests/cache.rs`.

### T1 validate

```bash
cargo test -p htui-core --all-features --no-fail-fast -- --test-threads=1
HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test -p htui-store --all-features --no-fail-fast \
  --test migrations --test connect --test pg_criteria --test cache --test pg_conformance \
  -- --test-threads=1
cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check
cargo clippy -p htui-core -p htui-store --all-targets --all-features -- -D warnings
cargo clippy -p htui-core -p htui-store -- -D warnings          # featureless (memory)
```

Do **not** build or gate the whole workspace in T1: `crates/htui` does not compile until T3 adds the
three `label::link_kind` arms (deviation 6). Look for `SIGABRT`/stack overflow in `pg_conformance`
(memory: htui-orch test stack headroom). Each new arm is boxed.

---

## T2 — MCP `item_link` speaks the new kinds (`crates/htui-mcp`)

### `src/tools/link.rs`

`kind` field (`:53`). The enum already grows through `LinkKind::ALL` (`kind_schema`, `:65`). The
doc does not fit one line, and a doc wrapped at 100 columns reaches the agent with a line break
(the comment at `:46`). So replace the `///` doc with a `description` attribute, as `to` does:

```rust
    // An attribute, not a doc: a doc wrapped at 100 columns reaches the agent with a line break.
    #[schemars(
        description = "How this item relates to `to`, read `<this item> <kind> <to>`: \
                       `blocked_by`: this item cannot start until `to` is finished; `origin`: \
                       this item was spawned by `to`; `relates`: see also; `supersedes`: this \
                       item replaces `to`; `parent`: this item is a part of `to`; `duplicates`: \
                       this item is the same as `to`; `implements`: this item carries out `to`'s \
                       decision. An item has at most one `parent` and one `duplicates`."
    )]
    #[schemars(schema_with = "kind_schema")]
    kind: LinkKind,
```

(`op` already pairs a description, its doc, with `schema_with`. If the tools/list test shows the
description missing, put both keys in one `#[schemars(schema_with = …, description = …)]`.)

Module doc (`:1-17`): append a paragraph:

```rust
//! MOD-150 (plan D9): `kind` is any [`LinkKind`], `parent`, `duplicates` and `implements`
//! included, and each goes live at once (pending links are MOD-155's). The store refuses a second
//! live `parent` or `duplicates`, a `parent` that closes a cycle and one that makes a chain deeper
//! than eight. Those refusals reach the agent unchanged as `refused: <sentence>` through
//! `store_error`, which names the item's current parent or canonical.
```

`call` stays as it is: the new refusals fall through to `other => store_error(other)`.

### `tests/tools_backlog.rs` (after `item_link_add_resolves_the_key_in_the_project`, `:628`)

`Leased::ana_2()` gives a step on `htui:ANA-2` (claimed, fence `Lease(owner)`), and that is the
whole parent-proposal path: from ANA-2, `to` is a bare key in `htui`. Import
`htui_core::store::traits::second_parent`.

1. `item_link_takes_the_new_kinds_live`: for `(to, kind, link_kind)` in
   `[("FEAT-1", "parent", Parent), ("FEAT-2", "duplicates", Duplicates), ("ANA-1", "implements", Implements)]`
   the answer `ok(…)` == `json!({"from": "ANA-2", "to": to, "kind": kind, "live": true})` and
   `fx.live_link(id, link_kind)` holds.
2. `item_link_second_parent_is_one_line_naming_the_current_parent`: add `parent FEAT-1` (ok), then
   `parent FEAT-3`. `refused(&answer) == format!("refused: {}", second_parent(ids::HTUI_ANA_2,
   ids::HTUI_FEAT_1))`, with no `'\n'` in it. `!fx.live_link(ids::HTUI_FEAT_3, LinkKind::Parent)`,
   while `fx.live_link(ids::HTUI_FEAT_1, LinkKind::Parent)` still holds.
3. `item_link_offers_the_seven_kinds`: `client.request("tools/list", json!({}))`, find
   `item_link` (as `item_status_offers_exactly_the_three_statuses` in `tools_search.rs:448`).
   `inputSchema.properties.kind.enum == json!(["blocked_by","origin","relates","supersedes","parent","duplicates","implements"])`.
   `…kind.description` equals the D9 text above verbatim and contains no `'\n'`.

No `tools/list` snapshot carries the `item_link` schema: a text search for the tool's description
finds only `link.rs:72`. The insta run should therefore move nothing; review it anyway.

### T2 commit and validate

Commit `feat(mcp): MOD-150 item_link documents and takes the seven link kinds`:
`crates/htui-mcp/src/tools/link.rs`, `crates/htui-mcp/tests/tools_backlog.rs`.

```bash
cargo test -p htui-mcp --all-features --no-fail-fast -- --test-threads=1
cargo insta test -p htui-mcp --all-features          # review every moved snapshot (expect none)
cargo clippy -p htui-mcp --all-targets --all-features -- -D warnings
cargo clippy -p htui-mcp -- -D warnings
```

`htui-mcp` does not depend on `crates/htui`, so it builds before T3 lands.

---

## T3 — Graph words for the new kinds (`crates/htui/src/ui`)

### `kit/label.rs` `link_kind` (`:283`)

```rust
        LinkKind::Supersedes => "supersedes",
        LinkKind::Parent => "part of",
        LinkKind::Duplicates => "duplicates",
        LinkKind::Implements => "implements",
```

Test `a_link_kind_reads_as_the_edge_says` (`:779-789`): the doc becomes "MOD-115 M4 D-1, MOD-150 D1:
…", and the pairs gain `("parent", "part of"), ("duplicates", "duplicates"), ("implements",
"implements")` after `("supersedes", "supersedes")`. `word_tables` (`:465`) needs no edit: it reads
`LinkKind::ALL`.

### `tabs/backlog/detail/graph.rs`

- `kind_width` doc (`:490`): "the widest of `label::link_kind`'s words (`blocked by`, `supersedes`,
  `duplicates`, `implements`, ten cells each)".
- `the_kind_column_is_the_widest_label` (`:2003`): the message becomes "`blocked by`, `supersedes`,
  `duplicates` and `implements`".
- New test beside `the_kind_column_is_padded_for_all_rows_or_none` (`:1817`):

```rust
    /// MOD-150 D1: the new kinds read as words in the kind column, `part of` padded to the ten
    /// cells `duplicates` and `implements` already fill, so the column keeps its width.
    #[tokio::test]
    async fn the_new_link_kinds_read_as_words_and_keep_the_column() {
        let shell = Shell::platform().await;
        let r = node(1, "R", 0);
        let p = node(2, "P", 1);
        let d = node(3, "D", 1);
        let i = node(4, "I", 1);
        let graph = LinkGraph {
            root: r.item_id,
            edges: vec![
                edge(&r, LinkKind::Parent, &p),
                edge(&r, LinkKind::Duplicates, &d),
                edge(&r, LinkKind::Implements, &i),
            ],
            nodes: vec![r, p, d, i],
        };
        assert_eq!(kind_width(), 10);
        let drawn: Vec<String> = lines(&rows(&graph, 3), &shell.projects, ROW, &shell.theme)
            .iter()
            .map(text)
            .collect();
        // Children sort by key bytes: D, I, P.
        assert_eq!(
            drawn[1..],
            [
                " \u{2192} duplicates D \u{25cb} open D title",
                " \u{2192} implements I \u{25cb} open I title",
                " \u{2192} part of    P \u{25cb} open P title",
            ]
        );
    }
```

### T3 commit and validate

Commit `feat(tui): MOD-150 Graph words for parent, duplicates and implements`:
`crates/htui/src/ui/kit/label.rs`, `crates/htui/src/ui/tabs/backlog/detail/graph.rs`.

```bash
cargo test -p htui --all-features --no-fail-fast --lib -- --test-threads=1 ui::kit::label ui::tabs::backlog::detail::graph
cargo insta test -p htui --all-features              # expect no moved snapshot (D1, D7)
cargo clippy -p htui --all-targets --all-features -- -D warnings
cargo clippy -p htui -- -D warnings
```

---

## After T2 and T3 (orchestrator)

```bash
bash scripts/gate.sh
cargo clippy --workspace -- -D warnings
cargo insta test --workspace --all-features
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

The host Postgres/Qdrant gate is still owed. Mark the PRD M1 row complete and add the HANDOFF phase
note (P1). HANDOFF `:56` lists the migrations; add `0030_link_kinds` there.

---

## Plan deviations

1. **Two case-count pins are missing from the Files table.** `crates/htui-core/tests/mem_store.rs:37`
   (`conformance::CASES.len()`, `277`) and `crates/htui-store/tests/pg_conformance.rs:59`
   (`EXPECTED_CASES: usize = 277`, plus the assert text at `:63-81`). The seven new cases make both
   `284`. They are in T1's crates, so T1 owns them.
2. **There are nine migration-count pins, not six.** The plan's six `Pending(29)` sites are right,
   but it misses `tests/connect.rs:156` (`pending, 29` in the `ConnEvent::MigrationsPending` arm),
   `tests/migrations.rs:118-119` (the applied-versions `vec![1, …, 29]` and its message) and
   `tests/migrations.rs:4452` (`common::count(&db.pool, "_sqlx_migrations")` == `29`).
3. **`ready_items` and `queue_rows` are not trait methods.** They are inherent on `MemStore`
   (`mem.rs:953`, `:1186`) and `PgStore` (`pg/read.rs:2274`, `:2539`), so a conformance case bound
   to `S: WriteStore` cannot call them. `new_kinds_never_gate_readiness` instead uses
   `ReadStore::items` with `ItemFilter { ready: Some(true) }` (the trait's readiness, the same
   `blocked_by`-only rule) and the full rows. A new `pg_criteria.rs` test,
   `new_link_kinds_never_gate_ready_items_or_queue_rows`, pins the inherent reads on both stores.
   That file is already in T1's set.
4. **Only one upstream `.sqlx` entry regenerates.** D6 says "both `.sqlx` entries regenerate".
   `CacheStore::upstream_summaries` runs `sqlx::query(AssertSqlSafe(format!(…)))`
   (`cache/read.rs:918`), a runtime query with no `.sqlx` entry. Only the Pg query
   (`query-5d8f75bb…json`) regenerates. The advisory lock's text is identical to `claim_command`'s
   (`pg/write.rs:9936`), so it reuses `query-751f836d…json`. Three new entries come from the
   propose checks.
5. **T1's validate list misses `--test pg_conformance`.** That test is the Postgres run of the
   seven new cases and holds the `EXPECTED_CASES` pin. Also, the `htui-core` filter `conformance`
   skips `mem_store.rs::demo_store_loads_the_fixture`, where the count pin lives. The T1 validate
   above runs the whole `htui-core` crate and adds `pg_conformance`.
6. **T1 leaves `crates/htui` uncompilable until T3 lands.** The plan's claim that the only
   exhaustive match is in `label::link_kind` (`ui/kit/label.rs:283`) is TRUE, and that is exactly
   why: adding the variants breaks that match. T1 and T2 validate per package (`htui-core`,
   `htui-store`, `htui-mcp` do not depend on `htui`). The workspace gate runs only after T3. This
   needs no change to the task split.
7. **The `tools/list` snapshots in the Files table do not exist.** A text search for `item_link`'s
   description (`"Proposes or withdraws a link"`) and for its kind doc finds only
   `crates/htui-mcp/src/tools/link.rs`. Expect no moved snapshot. T2 pins the schema in a test
   instead.
8. **Small additions to T1's set.** `crates/htui-core/src/store/mod.rs` re-exports the new
   constant and the four fns, following its `MAX_UPSTREAM_HOPS`/`link_outside_reach` convention.
   `model/link.rs` gains a `walks_upstream` unit test.

No decision (D1–D9) was found false.
