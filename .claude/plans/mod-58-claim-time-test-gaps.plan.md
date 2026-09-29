# Plan: MOD-58 — two claim-time test gaps

**Status: CONFIRMED by the maintainer 2026-09-28, as written.**

**Source**: `HANDOFF.md:269-275` (MOD-58, found 2026-09-26 from the MOD-7 milestone 3 review).
Requirement `R-ORCH-10`. Design authority: MOD-7 milestone 3 plan **D80**, **D81** and blueprint
**D94** (`.claude/plans/mod-7-capability-refusal.plan.md:173`;
`.claude/plans/mod-7-capability-refusal.blueprint.md:958`).

**Complexity: small, test-only.** No production code changes, no new trait method, no migration, no
`.sqlx` entry, no new dependency. Two files: `crates/htui-store/tests/pg_criteria.rs` and
`crates/htui-core/src/store/mem.rs`.

**Routing**: routed as plan by `/handoff-run MOD-58` (0 of C1–C4 fired). Ultracode not needed.
Staffing: the session model for every step, no model override on any agent, per the maintainer's
instruction for this session.

**Numbering**: MOD-58's own plan. Decisions **D1…D6**, risks **R-1…R-4**, tasks **T0**, **T1**.

**Gortex note**: `graphify-out/` does not exist. The Gortex MCP server answered one localization for
this item and then held a terminal contract that replayed the same payload for every later
navigation call, including a `read(operation:"source")` it had itself authorised; it never returned
the claim bodies or the `pg_criteria` helpers. Every tree fact below was therefore read directly
from the worktree at `68c058f` and carries a `file:line`.

---

## Summary

Two rules of the claim-time tag check are documented and implemented but not pinned by any test.

**(a) `NotClaimable` wins over `MissingTags` when the run targets another box.** Both stores decide
claimability before the tag check (`mem.rs:3602-3604` before `:3610`;
`pg/write.rs:3063-3065` before `:3071`), so a run whose `target_box_id` is not the claiming box is
refused even when its item needs tags the claiming box lacks. The shipped conformance case
`claim_run_fails_a_run_whose_item_needs_a_tag_the_box_lacks` (`conformance.rs:4716`) pins only the
*failed-run* half — a run that already answered `MissingTags` is not claimable again
(`:4802-4816`). The target-box half is unpinned: nothing asserts that such a run answers
`NotClaimable` and writes nothing.

**(b) D94 — a run with no item is never refused for tags.** `MemStore` uses `items.get`, not
`require_item` (`mem.rs:3610-3614`), mirroring the Postgres join that finds no row
(`pg/write.rs:3071-3086`). Unpinned in both stores.

Neither gap needs a shared trait method (D1), a migration, or a new dependency.

---

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **Both gaps are pinned per store, not in the shared conformance suite**: a `pg_criteria.rs` case for `PgStore` and in-file unit tests in `mem.rs` for `MemStore`. | The shared suite is `WriteStore`-generic and has no way to mint a second `box` row: `traits.rs` has no box-creating method (`upsert_agent_box` writes `agent_box`, not `box`; `record_box_probe` updates an existing row). Adding one would be a cross-store change to the shared conformance surface, which the maintainer holds closed for this item. The item text already names this placement. |
| D2 | **The second `box` row is planted by the test itself, not through the store.** Postgres: raw `INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version)` with the *untyped* `sqlx::query(...).bind(...)` form. `MemStore`: insert a cloned fixture `BoxRow` with a fresh id and hostname into `State.boxes` through the private `MemStore::write`. | `create_run` requires the target box to exist in both stores (`mem.rs:3537-3543`; `REFERENCES box(id)` at `0001_init.sql:455`), so a run cannot be aimed at a box that is not there. The untyped form is the existing precedent (`connect.rs:147`, `cache.rs:1865`) and keeps `.sqlx/` at 268. **The hostname is `'elsewhere'` for legibility, not because a constraint forces it:** `UNIQUE (user_id, hostname)` from `0001_init.sql:73` was dropped by `0005_box_identity.sql:15` (ANA-16 C4: it made a renamed box a duplicate primary key). |
| D3 | **The (a) case asserts the refusal *and* that nothing was written**: `Claim::NotClaimable`, the run row equal to what `create_run` returned, and the item still `Status::Queued`. | A refusal that wrote nothing is half the rule (the module doc at `mem.rs:3583-3585` says so). Asserting the verdict alone would still pass if the tag branch ran first and the run were then rolled back. |
| D4 | **D94 is pinned twice on `MemStore` (`item_id: None`, and `item_id` naming a row that is gone) and once on Postgres (a `queued` run inserted with `item_id = NULL`).** | `run.item_id` is `ON DELETE CASCADE` (`0001_init.sql:450`), so in Postgres a deleted item takes its run and the "row is gone" half is unreachable there. `MemStore` can express both, and both are the two arms of D94's sentence. |
| D5 | **The Postgres D94 run is inserted with the untyped `sqlx::query` form**, `kind = 'graph'`, `graph_snapshot = '{}'::jsonb`. | `ck_run_graph_snapshot` (`0003_orchestration.sql:47-48`) only requires a graph run's snapshot to be non-NULL; the check is `NOT VALID`, so it is enforced for new rows and the value's shape is not tested. Keeping the insert out of `query!` keeps `.sqlx/` at 268 (D2). |
| D6 | **The missing tag is `cuda`, which the fixture box lacks** (`probed_tags: ["rust", "msvc", "cmake"]`, `declared_tags: ["gpu"]` — `fixtures.rs:471-472`). | The (a) case must fail the box rule while the tag rule is genuinely armed, or it passes for the wrong reason. `cuda` is the tag the existing `mem.rs:9004` case already uses for that. |

---

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-store/tests/pg_criteria.rs` | edit | T0 | two `#[tokio::test]` cases: (a) a missing-tags run aimed at a second box, (b) a `queued` run with no item |
| `crates/htui-core/src/store/mem.rs` | edit | T1 | two `#[tokio::test]` cases in the in-file `mod tests` (`:5878`): both halves of (a) and D94 |

**Not touched, on purpose:** `crates/htui-core/src/store/traits.rs` (D1 — no trait method);
`crates/htui-core/src/store/conformance.rs` and its `CASES` (D1 — the shared suite cannot mint a
second box, and its count pin must not move for a test it does not host);
`crates/htui-store/src/pg/write.rs` and `crates/htui-core/src/store/mem.rs`'s production half (the
behaviour is correct as built — this item pins it, it does not change it);
every migration and `cache_migrations/` (D2, D5 — no schema change, and `0009` belongs to the MOD-9
lane); `crates/htui-store/.sqlx/` (D2, D5 — untyped queries only); `HANDOFF.md`, `DECISIONS.md`,
`docs/**`.

**One out-of-plan correction, maintainer-approved 2026-09-28.** The review found that the D94
comment inside `State::claim_run` (`mem.rs:3606-3609`) repeats the false claim the tests were written
to disprove — it names a chat run as a run with no item, but `start_chat_run` inserts at `running`,
so a chat run is `NotClaimable` at the status half and never reaches the tag query. A pin whose own
production comment contradicts it is the worst place to leave that, so the comment is corrected. It
is a comment: no production line's behaviour changes, and the "not touched, on purpose" rule above
holds for everything except this.

---

## Tasks

**Independence.** T0 and T1 are file-disjoint (`crates/htui-store/tests/pg_criteria.rs` against
`crates/htui-core/src/store/mem.rs`), verified above. They are **still run one at a time** on the
main thread: they share one worktree, one `.git/index` (two agents committing concurrently contend
on `index.lock`, and project memory is explicit that implementer work must be committed
incrementally) and one `target/` directory (cargo takes a build lock, so the second lane would
queue anyway). The gates are re-run on the real tree after each task.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `crates/htui-store/tests/pg_criteria.rs` | independent of T1; run second |
| T1 | `crates/htui-core/src/store/mem.rs` | independent of T0; run first (no server needed) |

**Conditions binding on both implementers:**
1. **No production code changes.** If a test cannot be written without changing `claim_run`,
   `create_run` or a trait, stop and report — that is a finding about the behaviour, not a licence
   to edit it.
2. **No migration, no `.sqlx` entry.** If an insert cannot be written with the untyped
   `sqlx::query` form, stop and report.
3. **No `HANDOFF.md` edit.** Bookkeeping is the main thread's (close-out).
4. **Commit incrementally**, staging only your own path. Do not stash. Do not push.
5. Tests only — no `todo!()` scaffolding, since nothing new is written to make a test pass.

### Task 1: the `MemStore` pins (D1, D2, D3, D4, D6)

- **File**: `crates/htui-core/src/store/mem.rs`, inside `mod tests` (`:5878`), after
  `claim_run_refuses_an_overlapping_scope_and_a_full_box` (`:7757`).
- **Case 1 — `a_missing_tags_run_aimed_at_another_box_is_not_claimable`**
  1. `let store = MemStore::demo();`
  2. Plant the second box: read the fixture's `ids::BOX` row, clone it, give it `BoxId::new()` and
     a distinct `hostname` (`"elsewhere"`, the word `connect.rs:150` already uses), and insert it
     into `State.boxes` through `store.write(...)` (private, `:743`; reachable from the child
     module).
  3. Mint an item with `required_tags: vec!["cuda".to_owned()]` — the shape at `mem.rs:8996-9010`.
  4. `create_run(NewRun { target_box_id: other, ..graph_run(item, ids::PROJECT_HTUI, Vec::new()) })`
     — `graph_run` (`:7625`) is the file's helper; the struct-update override replaces the
     `ids::BOX` it hard-codes. The row `create_run` returns is kept for D3's comparison.
  5. `claim_run(run, ids::BOX, owner, at, until)` must be `Claim::NotClaimable`, and the box rule
     must be the reason: the claim names `ids::BOX`, which is the box that lacks `cuda`.
  6. Assert `store.run(run)` equals the `create_run` row (D3) and the item is still
     `Status::Queued` (D3). Assert the run's `target_box_id` is the second box, so the case cannot
     pass by aiming at `ids::BOX`.
- **Case 2 — `a_run_with_no_item_is_never_refused_for_tags`** (both arms of D94)
  1. `item_id: None` — create a run normally, then `store.write(|s| s.runs.get_mut(&run).unwrap().item_id = None)`.
     Claim from `ids::BOX`: `Claim::Admitted`, and the run reads back `running` with
     `executing_box_id == Some(ids::BOX)`.
  2. `item_id` naming a row that is gone — create a run on a second item, then
     `store.write(|s| { s.items.remove(&item); })`. Claim: `Claim::Admitted`. This is the arm that
     would answer `NotFound { entity: "item" }` if `claim_run` used `require_item`.
  - Use separate items and separate runs for the two arms, so the two share no state. (The first
    arm's item is *not* moved to `in_progress` by its claim — that run has `item_id: None`, so the
    admitted branch's item transition is skipped — but a second item keeps the arms independent
    whatever that branch does.)
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`;
  `cargo clippy -p htui-core --all-features --all-targets -- -D warnings`;
  `cargo fmt --all -- --check`.

### Task 0: the `PgStore` pins (D1, D2, D3, D4, D5, D6)

- **File**: `crates/htui-store/tests/pg_criteria.rs`, after
  `finish_run_holds_the_item_while_another_run_is_live` (`:3676`).
- **Boilerplate**: the file's own — `let Some(db) = common::demo_db().await else { return; };` at
  the top (`:3557`) and `db.drop_db().await;` at the bottom (`:3675`). `db.store` is the `PgStore`,
  `db.pool` the raw pool. `race_run` (`:902`) is the file's `NewRun` helper. `race_item` (`:64`) is
  `fn race_item(kind_id: ItemKindId, title: &str) -> NewItem` — the `kind_id` is mandatory (omit it
  and the insert is a `23503`) and it hard-codes `required_tags: Vec::new()`. The attribute is
  `#[tokio::test(flavor = "multi_thread")]`, as in the other 35 cases in the file.
- **Case 1 — `a_missing_tags_run_aimed_at_another_box_is_not_claimable`**
  1. Plant the second box with the untyped form, copying the column list and the
     `(SELECT id FROM app_user ORDER BY created_at, id LIMIT 1)` sub-select from `connect.rs:147-151`,
     with `hostname = 'elsewhere'` (legibility — the unique constraint that once forced it is gone,
     D2). The seven columns named are exactly the `NOT NULL`-without-default columns of
     `0001_init.sql:53-74`.
  2. Mint an item with `required_tags: ["cuda"]` — `race_item(ids::KIND_HTUI_FEAT, title)` with the
     tags overridden — and `create_run(NewRun { target_box_id: other, ..race_run(item) })`. Keep the
     returned row.
  3. **Before the claim, assert the tag rule is armed**: `db.store.missing_tags(item, ids::BOX)` is
     `["cuda"]`. Without this, a `required_tags` override that silently did nothing would leave the
     case answering `NotClaimable` for the wrong reason. The committed MemStore case has this guard
     at `mem.rs:7928-7935`; copy its shape.
  4. `claim_run(run, ids::BOX, …)` must be `Claim::NotClaimable`; the run row must equal the
     `create_run` row and the item must still be `Status::Queued` (D3).
- **Case 2 — `a_run_with_no_item_is_never_refused_for_tags`**
  1. Insert the run directly: untyped `INSERT INTO run (id, project_id, item_id, kind, mode,
     status, target_box_id, graph_snapshot, started_by, queued_at) VALUES ($1, $2, NULL, 'graph',
     'manual', 'queued', $3, '{}'::jsonb, $4, $5)`. `repo_scope` defaults to `'{}'`
     (`0003_orchestration.sql:36`), which by hazard H-10 overlaps nothing, so the claim can only be
     refused by the tag rule — and the tag rule is what the case says is never reached. The
     `NOT NULL`-without-default columns are `project_id`, `kind`, `mode`, `target_box_id` and
     `started_by`, all supplied.
  2. `claim_run(run, ids::BOX, …)` must be `Claim::Admitted`; the run reads back `running` with
     `executing_box_id == Some(ids::BOX)`.
- **Validate**:
  `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
     cargo test -p htui-store --test pg_criteria --all-features -- --test-threads=1`;
  `cargo clippy -p htui-store --all-features --all-targets -- -D warnings`;
  `cargo fmt --all -- --check`.

---

## Test plan

TDD does not apply in its usual shape: both tasks are tests, and the behaviour they pin is already
implemented and believed correct. Each task's first commit is therefore the finished cases; the
red-then-green discipline is replaced by the mutation check below.

**Mutation check — the only honest proof that a pin bites.** After each task's cases pass, break the
rule on purpose and confirm the case fails, then revert:
- T1 and T0, case 1: move the target-box check *after* the tag check in the store under test. The
  case must fail with `MissingTags` where it asserts `NotClaimable`. **The move has to place the
  `return Ok(Claim::NotClaimable)` after the whole `if !missing.is_empty() { … return
  Ok(Claim::MissingTags { missing }); }` block, not merely after the *computation* of `missing`** —
  hoisting the predicate into a binding and returning after the `let` still leaves the `MissingTags`
  write block after the return, so nothing is written and the case still passes. That weaker form
  is not a mutation and would report a false green (found by the T1 implementer).
- T1 and T0, case 2: swap `items.get` for `require_item` (`mem.rs:3612`). **The Postgres half is not
  a swap** — the join at `pg/write.rs:3075` is already an inner join and there is no `require_item`
  equivalent in the SQL. D94 there is the *absence* of a row-level demand, so the mutation is to
  **add** one: a `require_item`-equivalent between `:3063` and `:3071` that answers
  `NotFound { entity: "item" }` when `item_id` is NULL. The case must then fail with that error.
  (Found by the blueprint pass; the plan's original "swap the join for an inner join" was not
  expressible and would have produced a false green.)

Report the observed failure line for each of the four mutations in the task's report. A pin that
cannot fail is not a pin.

**What the `MemStore` D94 mutation actually proved.** The reported result for the T1 case-2 swap
(`items.get` → `and_then(|item| self.require_item(&item).ok())`) is accurate as a fact but was
reported as if it caught the whole case. It is caught by the **second arm only**: with
`item_id: None`, `and_then` short-circuits before the closure ever runs, so the first arm is
unaffected by that shape of mutation. Arm A needs the demand written in the `map`-shaped form —
`claimed.item_id.map(|item| self.require_item(&item)).transpose()?` or equivalent, which *is*
evaluated for `None` — to fail. The case as a whole does bite, which is why it stands; the claim
is narrowed here so nobody later reads the first arm as independently guarded against a
row-level demand.

**Coverage map.** Four cases, two stores, two rules:

| Rule | `MemStore` | `PgStore` |
|---|---|---|
| (a) `NotClaimable` beats `MissingTags`, writes nothing | T1 case 1 | T0 case 1 |
| (b) D94, no item | T1 case 2, both arms | T0 case 2 |

**Count pins that move:** none. Store `CASES` is unchanged (D1 — no conformance case is added);
`.sqlx` stays 268; migrations stay `0001`..`0007` and `0008` stays the next free number; no snapshot
moves; no `StoreRequest` / `StoreReply` variant is added.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — The Postgres case only runs where a server and `HTUI_TEST_DATABASE_URL` are up, so a `PgStore` regression in this rule is invisible in the default suite | Medium | Accepted: it is the shape `pg_criteria.rs` exists for. The `MemStore` cases run in the default suite, so the rule itself is pinned somewhere without a server |
| **R-2** — The in-file `MemStore` cases reach private state (`State.boxes`, `State.items`, `MemStore::write`), so they can construct a state the store cannot reach | Medium | Every step goes through `State`'s own invariants: the second box is a clone of a real row (D2), and the two D94 arms are exactly the two shapes D94 names. A future change that made either shape unreachable would have to change the test, which is the signal wanted |
| **R-3** — A `pg_criteria` failure is read as a real failure when the dev Postgres is in recovery (`SQLSTATE 57P03`) | High on this box | Re-run the case alone before believing it; `df -h /` first (project memory: the dev Postgres crash-loops under disk pressure) |
| **R-4** — The maintainer would rather have one shared conformance case and pay for a `WriteStore` method | Low | D1 states the cost plainly; the maintainer holds the shared surface closed for this item, and a trait method would touch `mem.rs`, `pg/write.rs`, `writer.rs` and the two test doubles |
| **R-5** — Known cosmetic asymmetry: the two stores' second boxes are not the same shape of row | Low | Accepted. `MemStore` clones the fixture's `BoxRow` and so carries its `probed_tags`/`declared_tags`; the Postgres box is planted by an untyped `INSERT` that leaves both to their `'{}'` defaults and `machine_fingerprint` NULL. Irrelevant to the rule under test — it reads the *claiming* box (`ids::BOX`) in both cases — but a future case that cared about the second box's tags would have to raise the fidelity of the planted row |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test -p htui-core --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --test pg_criteria --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
# the prepare check needs a scratch database migrated through 0007 (project memory: the compose
# `htui` database is empty):
cd crates/htui-store && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check sqlx migrate run --source migrations && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features
ls crates/htui-store/.sqlx | wc -l                      # 268, unchanged
cargo doc --workspace --no-deps --keep-going            # exactly the six baseline errors
git diff --stat 68c058f -- crates/htui-store/migrations crates/htui-store/cache_migrations \
  crates/htui-core/src/store/traits.rs crates/htui-core/src/store/conformance.rs \
  crates/htui-store/src/pg/write.rs                     # empty
```

`--test-threads=1` is not optional (the keyring fake is process-wide). Postgres cases get the
`USERNAME=htui-ci` prefix. Before believing a Postgres failure, `df -h /`, then re-run the case
alone.

## Acceptance

- [ ] A `queued` run whose `target_box_id` is a second, real `box` row and whose item needs a tag
      the claiming box lacks answers `Claim::NotClaimable` on both stores, with the run row
      unchanged and its item still `queued`.
- [ ] A `queued` run with no item is admitted on both stores, and on `MemStore` so is a `queued`
      run whose `item_id` names a row that is no longer there.
- [ ] Each of the four pins fails when its rule is broken (the mutation check, above), and the
      failure is reported.
- [ ] No production code, trait, migration, `.sqlx` entry, dependency or count pin moved.
- [ ] `validate-workflow-docs.sh` exits 0 at close-out.

## Where the HANDOFF or the tree disagree

- `HANDOFF.md:272-274` — "The shared store conformance has no trait method to make a second box"
  is **confirmed**: `traits.rs` has no box-creating method. The consequence the item draws (a
  `pg_criteria` case plus a `mem.rs` unit test) is what D1 adopts.
- `HANDOFF.md:274` — "(b) fits either". This plan does **both** stores. The reason is not
  symmetry: the `MemStore` case runs in the default `cargo test -p htui-core` suite with no server,
  so the rule stays pinned even where the Postgres suite never runs.
- `HANDOFF.md:271-272` — D81's `NotClaimable` "cancelled-run half" is named; what
  `conformance.rs:4802-4816` actually pins is the **failed**-run half (a run that already answered
  `MissingTags`). Same rule, one status over; the case added here is the target-box half, which is
  unpinned either way.
- **D94's two arms are not symmetric across stores** (D4). The HANDOFF sentence "a run with no item
  (or whose item row is gone)" reads as one test; on Postgres the second arm is unreachable, because
  `run.item_id` is `ON DELETE CASCADE`.

## Claims to verify

Every claim is a statement about the tree at `68c058f`, checked in the table below.

## Verified claims

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `MemStore::claim_run` returns `NotClaimable` when `target_box_id != box_id`, before the tag check | verified | `mem.rs:3602-3604` returns; the tag check starts at `:3610` |
| 2 | `PgStore::claim_run` has the same predicate in the same position | verified | `pg/write.rs:3063-3065`; the tag query starts at `:3071` |
| 3 | A run cannot be aimed at a box that has no row, in either store | verified | `mem.rs:3537-3543` (`references_no_row("run.target_box_id", …, "box")`); `0001_init.sql:455` `target_box_id UUID NOT NULL REFERENCES box(id)` |
| 4 | `NewRun.item_id` is a non-optional `ItemId`, so a run with no item cannot be minted by `create_run` | verified | `model/run.rs:358-359` |
| 5 | `start_chat_run` mints a `running` chat run with `item_id` NULL, so it is `NotClaimable` by status and is not a D94 vehicle | verified | `pg/write.rs:1333-1335` (`'chat' … 'running' … NULL`); `claim_run` checks status first (claims 1, 2) |
| 6 | On Postgres a deleted item takes its run, so D94's second arm is unreachable there | verified | `0001_init.sql:450` `item_id UUID REFERENCES item(id) ON DELETE CASCADE` |
| 7 | `pg_criteria.rs` has the per-case database boilerplate and raw pool access | verified | `common::demo_db()` at `:3557`; `db.drop_db()` at `:3675`; `&db.pool` at `:651`, `:3559` |
| 8 | A second `box` row is insertable from a test with the untyped `sqlx::query` form, so `.sqlx/` does not move | verified | `connect.rs:147-151`, `cache.rs:1865`; only `query!` macros write `.sqlx` entries |
| 9 | The in-file `mem.rs` test module can reach `MemStore::write` and `State.boxes` | verified | `mem.rs:5878` `#[cfg(all(test, feature = "test-support"))] mod tests` is a child of `mem`; `write` at `:743` is private to it; `boxes` at `:91` |
| 10 | A second box needs only id, user, hostname, os_family, os_version, arch, htui_version | verified | `0001_init.sql:53-74` (every other column has a default; `machine_fingerprint` is nullable, `0005_box_identity.sql:17-18`). **Amended at blueprint:** `UNIQUE (user_id, hostname)` is gone — `0005_box_identity.sql:15` drops it — so a distinct hostname is a readability choice, not a constraint |
| 11 | The Postgres D94 insert needs no valid snapshot value | verified | `0003_orchestration.sql:47-48` `CHECK (kind <> 'graph' OR graph_snapshot IS NOT NULL) NOT VALID` |
| 12 | The target-box half of the rule is unpinned today | verified | `conformance.rs:4716-4879` asserts the failed-run half (`:4802-4816`); `:4287-4469` asserts `NotClaimable` only for a status reason (`:4452`) |
| 13 | D94 is unpinned in both stores | verified | `mem.rs:3606-3614` and `pg/write.rs:3067-3086` carry the rule; no case in `conformance.rs`, `pg_criteria.rs` or `mem.rs`'s tests builds a run with no item |
| 14 | No `WriteStore` method creates a `box` row | verified | `traits.rs` box-named methods: `upsert_agent_box:335` (writes `agent_box`), `set_agent_box_quota:365`, `record_box_probe:385` (updates), `boxes:393` (read), `edit_box:412`, plus the path writers — none inserts into `box` |
| 15 | `cuda` is a tag the fixture box lacks | verified | `fixtures.rs:471-472` `probed_tags: ["rust","msvc","cmake"]`, `declared_tags: ["gpu"]`; the existing case at `mem.rs:9004` uses `cuda` for this |
| 16 | `.sqlx` holds 268 entries and the last migration is `0007` | verified | `ls crates/htui-store/.sqlx \| wc -l` → 268; `migrations/` holds `0001_init`..`0007_skill_attachments`, so `0008` is the next free number |
| 17 | An empty `repo_scope` overlaps nothing, so the D94 Postgres case cannot be refused for overlap | verified | `mem.rs:3657-3660` (hazard H-10) and the same filter in `pg/write.rs:3150-3152` |
