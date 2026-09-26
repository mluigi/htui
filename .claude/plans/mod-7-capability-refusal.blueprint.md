# Blueprint: MOD-7 milestone 3, "a mismatch is refused by name"

**Status**: **proposed** (2026-09-26). Plan deviations P-1 to P-7 (§0) and decisions D90–D103 (§9)
are this blueprint's. Where a deviation says **Blocker**, the plan read literally either fails its
own gate or asserts something the tree cannot show. The Fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-7-capability-refusal.plan.md` at `5113d00`, status **confirmed**
(maintainer adopted OQ-20..OQ-25 as recommended), fact-checked. Its D75–D89, its "Verified claims"
table and every "(amended at fact-check)" note are binding and are not reopened here. PRD D0–D7 win
over this blueprint where they disagree. Tasks are cited as "milestone 3 T*n*" outside this file.

**Verified at**: HEAD `5113d00`, branch `mod-7-m3`. `git diff --stat 98e6d2f HEAD -- crates
Cargo.toml Cargo.lock` is empty, so the plan's line numbers (taken at `98e6d2f`) still hold. Every
anchor below was located through Gortex (`search`, `read`, `relations`) and re-read at its line.
**Line numbers are pre-edit**: a citation into a file a task edits moves after that task's first
commit. `crates/htui-store/.sqlx/` holds **264** files, `crates/htui/tests/snapshots/` **87**,
migrations are `0001`..`0007`, and `df -h /` shows **63 GB free (86 % used)**: tighter than
milestone 2's 92 GB, so check it before each wave (project memory: `target/` fills the disk and the
dev Postgres crash-loops).

**Graphify**: `graphify-out/` does not exist in this checkout; nothing here comes from it.

**Coupling verdict.** Wave 1 (T0 ∥ T1) stands as the plan has it. Wave 2 keeps T2 ∥ T3 for
*authoring*, but **P-2** adds a behavioural dependency the plan missed: T3's second test can only go
green on a tree that carries T2. So the merge order inside Wave 2 is fixed as **T2, then T3**, and
T3 runs its second test's gate only after rebasing onto the merged T2 (D90). No file moves between
tasks; every task's file set is the plan's, with the additions named in §0 all falling inside a
file the same task already owns.

**Scope**:
- **Order**:
  1. Wave 1: T0 (worktree A) ∥ T1 (worktree B).
  2. Merge T0, then T1, re-running the `htui-core`, `htui-store`, `htui-orch` and `htui` gates on
     the real tree after each merge.
  3. Wave 2: T2 (worktree C) ∥ T3 (worktree D).
  4. Merge T2. T3 rebases onto it, runs its full gate, and merges.
  5. The workspace gate (§8), then the optional live check (plan §Validation).
- **No migration**: the next is still `0008`.
- **One new `Claim` variant** (`MissingTags`), `Claim` loses `Copy`. **One new pure fn**
  (`missing_tags_failure`). **One new `GraphSource` method** on all four implementations. **One
  new `RunFailure` and one new `EngineError` variant.**
- **Request enums**: unchanged (`StoreRequest` 68, `StoreReply` 39).
- **Pins that move**: store `CASES` 74 → 76 (T0); `htui-orch` `CASES` 70 → 71 (T1) → 72 (T2);
  `.sqlx` 264 → **267** (T0); `GraphSource` methods 6 → 7 (T1). Snapshots 87 → 87.

**House style (carried)**:
- `unsafe_code = "forbid"`, `missing_docs` warns, clippy `all` with `-D warnings` in the gate,
  pedantic **not** enabled. rustdoc denies broken **and private** intra-doc links: a `pub` item's
  doc must not link to a private fn (`refuse_missing_tags`), and a doc written before the target
  exists uses plain backticks.
- Nothing sets `updated_at` by hand on Postgres (the `set_updated_at` trigger does); `MemStore`
  stamps `now` as its twin.
- Implementers commit incrementally, staging their own paths only (never `-A`, never `stash`).
  Every commit compiles; a red commit uses `todo!()` bodies only where no existing path calls them.
- Every gate is re-run with `--test-threads=1` on the real tree after the merge (the keyring fake
  is process-wide; project memory).

---

## 0. Plan deviations (found against the tree)

| # | Blocker? | Plan says | Tree at `5113d00` | Fix |
|---|---|---|---|---|
| **P-1** | **Blocker** (T3's second test would assert a line that never appears) | D89, T3's `a_claim_time_refusal_is_not_requeued_on_postgres`: "its run is then `failed` … and **the status line carries the sentence**". | The claim retry is `reclaim`, spawned by `claim_queued` on `ctx.unaddressed("claim_retry")` (`crates/htui/src/run_worker.rs:1887-1903`), whose `addr` is `None` ("a claim retry … answer[s] nobody and only publish[es]", `:1693-1695`). `TaskCtx::refuse` (`:1723-1732`) publishes a `FrameKind::Error` and calls `answer`, which is a no-op without an `addr` (`:1701-1709`). `App.status` is fed only by `StoreReply::Failed` (`crates/htui/src/app/update.rs:175`, `"{request}: {message}"`). A claim-time refusal found by a retry therefore never reaches the status line, with or without T2. | D91: T3's second test asserts the **note** instead, `missing tags: gpu` on the item, which only T2's `Engine::claim` arm writes. It also asserts that the status line is `None` after the retry, which is the unaddressed task's contract. |
| **P-2** | **Blocker** (T3's gate cannot be green in its own worktree) | Build coupling: "T3 needs T0's claim check and T1's enqueue check. **Both parallel markings stand.**" | Without T2, `Engine::claim` answers `ClaimRefused { claim: MissingTags }`. `reclaim`'s `ClaimRefused` arm (`run_worker.rs:1944-1947`) re-queues it with a `tracing::debug!` and writes no note, so the run is `failed` and the item `blocked` (T0), but no note exists. The note is the only row that separates "mapped to `MissingTags`" from "re-queued". | D90: T2 and T3 are still authored in parallel. **Merge T2 first.** In its own worktree T3 gates with `--skip a_claim_time_refusal_is_not_requeued_on_postgres`, then rebases onto the merged T2 and runs the full `runs_pg` gate before its own merge. T3's two tests are two commits (§5.4). |
| **P-3** | Non-blocker (a stale comment would survive) | D86: reword "criterion 14's `Unblock` half" in the MOD-4 case's doc (`conformance.rs:5197-5199`) and the `CASES` preamble (`:330-333`). | There is a **third** site: the `CASES` entry comment `// Criterion 14's \`Unblock\` half (\`:2121-2122\`, plan D161): rung 4's blocked item reopens.` at `crates/htui-orch/src/conformance.rs:481`. | T1 rewords all three (§3.7). |
| **P-4** | Non-blocker (stale counts in docs) | D75: the through-the-trait test's doc at `fake.rs:2117` says "six". | `impl GraphSource for MemStore`'s own doc also says "The six inherent orchestration reads" (`crates/htui-orch/src/fake.rs:830`) and "calls all six through the trait" (`:843`). | T1 changes all three to "seven". |
| **P-5** | **Blocker** (D87's case would never see an overlap) | D87: "Two items minted with overlapping `touched_paths` (`mint_feat`)"; the second is refused `Claim::Overlaps`. | The demo project has no repo, so a resolved `repo_scope` is empty. An empty scope shares no repo and is never refused for overlap (hazard H-10, `mem.rs:3593-3596`; `pg/write.rs:2984-2985`). `overlapping_touched_paths_serialise` calls `primary_repo(&orch)` first for exactly this reason (`conformance.rs:3562`). | T2's case calls `primary_repo(&orch).await` before minting (§4.5). |
| **P-6** | Non-blocker (precision) | D89: "`StartRun` through the worker puts `item …: missing tags: docker, vulkan` on the status line". | The status line is `"{request}: {message}"` (`app/update.rs:175`), and the worker's request name for `StartRun` is `"start_run"` (`run_worker.rs:67`). | T3 asserts `start_run: item <FEAT-3 uuid>: missing tags: docker, vulkan` byte for byte (§5.2). |
| **P-7** | Resolved, not a deviation | D89: "If `Stack`'s seed cannot give the two items a shared repository, the test is dropped." | `runs_pg.rs::seed` always creates a **primary** repo for `PROJECT_HTUI`, so two undeclared items both hold the whole repo and overlap. | The second test is kept (D99). |

### 0a. Hazards the plan lists, each with its guard

| # | Hazard | Guard (who, how it fails loudly) |
|---|---|---|
| **H-1** | `ck_run_graph_snapshot` (`0003_orchestration.sql:47-48`, `CHECK (kind <> 'graph' OR graph_snapshot IS NOT NULL) NOT VALID`) is checked on every `UPDATE`, so a snapshot-less seeded `graph` run (`RUN_2`) cannot be failed on Postgres. | T0: every run the new store cases fail through `claim_run` is minted by `create_run(new_run(..))`, and `new_run` always sets `graph_snapshot: run_snapshot()` (`store/conformance.rs:3883-3895`). Neither case body names `ids::RUN_2`, and the reviewer greps both bodies for it. The Postgres `pg_conformance` gate runs both cases; a violation is a `23514` `check_violation`, which fails the case by name. Production is unaffected: `Engine::enqueue` always passes a snapshot. |
| **H-2** | `Claim` loses `Copy`; `pg_criteria.rs:694` moves both verdicts and `:709` reads `one` again (E0382). | T0 fixes both lines **in the same commit that drops `Copy`** (§2.1, commit (a)), so no commit of T0 fails to build a test target. T0's gate runs `cargo check --workspace --all-features --all-targets`, which compiles every crate's test targets against the new `Claim` (`htui-store` tests, `htui-agent`'s two `claim_run` forwards, `htui-orch`'s `ClaimRefused`, `htui`). The Gortex search for `Claim::` found no other copy site: `EngineError` derives only `Debug` and `thiserror::Error` (`command.rs:309`), and every other use moves, borrows or compares. |
| **H-3** | The run worker re-queues exactly `EngineError::ClaimRefused` (`run_worker.rs:1944`, `:2162`), so a claim-time `MissingTags` reported as `ClaimRefused` would be re-queued. | T2 matches `Claim::MissingTags` **before** the `ClaimRefused` catch-all in `Engine::claim` (§4.3). T2's engine test asserts the error is not `ClaimRefused`. T3's second test asserts the note, which only that arm writes (D91). The reviewer confirms both worker arms still name `ClaimRefused { .. }` only, and T3 leaves `run_worker.rs` alone. The T0-merged-before-T2 window (plan R-34) is covered by `reclaim`'s early return on a non-`queued` run (`:1933-1935`); no release is cut mid-wave. |
| **H-4** | The `htui-orch` count lives in prose as well as in two pins: `conformance.rs:288` ("Seventy"), `:289` (the pin test's name), `:291` (the sum), `:507` ("seventy-arm `match` … all seventy"), the pin test's message (`:5519-5538`), `tests/fake_conformance.rs:15-17`. Each new case also needs an arm in `fn case` (`:513`). | T1 and T2 each follow §3.7's checklist. A missing arm panics `every_case_name_dispatches` (`:5545`) by name. After each task, `rg -n 'eventy' crates/htui-orch` must print only the new wording (T1: "Seventy-one", "seventy-one-arm"; T2: "Seventy-two", "seventy-two-arm"), and `rg -n 'cases_are_unique_and_seventy\|cases_len_is_seventy' crates` must print nothing (D96 renames them). |
| **H-5** | `.sqlx` regeneration needs a scratch database migrated through `0007`; the compose `htui` database is empty (project memory). | §2.6's recipe on a freshly recreated `htui_prepare_mod7m3`, then `prepare --check`. Guard: `git status --porcelain crates/htui-store/.sqlx` shows **exactly three `??` lines and no `M` or `D`**, and `ls crates/htui-store/.sqlx \| wc -l` prints **267**. |
| **H-6** | *(found here)* `store::conformance`'s `every_cross_referenced_test_name_exists` (`crates/htui-core/src/store/conformance.rs:10134`) checks **every backticked span in the file**, assertion messages included. A bare snake_case name with four or more underscores must be a fn in `conformance.rs` or `mem.rs`, and a `<file>.rs::<name>` span whose file is not `conformance`, `mem`, `pg_criteria` or `box_identity` **panics**. | T0's case docs and messages cite **no** `htui-orch` or `htui` test name in backticks (for example not `a_capability_refusal_at_claim_fails_the_run_by_name`, not `runs_pg.rs::…`); they name those halves in prose ("the `htui-orch` claim-time case"). `ck_run_graph_snapshot` has three underscores and is safe. The `htui-core` gate runs the scanner. |
| **H-7** | *(found here)* The engine's enqueue check also runs for every `StartRun` in the `htui` harness suites, over fixture items the plan's R-37 did not list: `TOOL-1` requires `docker` and Vulkan `FEAT-1` requires `gpu, vulkan` (`fixtures.rs:973`, `:1038`), and the demo box lacks `docker` and `vulkan`. | Both items are seeded `awaiting_approval` and `in_progress`, and D79 checks only an `open` or `failed` item, so a `StartRun` on either still meets `create_run`'s existing refusal. T1's `enqueue_checks_tags_only_where_create_run_would` pins that guard (D98). T1's gate runs the whole `htui` suite, Postgres included. |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (each compiles) | Gate |
|---|---|---|---|
| T0 claim check | htui-core, htui-store (worktree A) | 3 (§2.7) | `htui-core`; `cargo check --workspace --all-features --all-targets`; Postgres `pg_conformance` + `pg_criteria`; `htui-orch`; prepare + `--check`; `.sqlx` = 267 |
| T1 seam + enqueue | htui-orch, htui (worktree B) | 2 (§3.9) | `htui-orch`; `htui` with the Postgres suites; clippy on both |
| merge | — | T0, then T1 | after **each**: the gates of the crates it touched, on the real tree |
| T2 engine claim | htui-orch (worktree C) | 3 (§4.7) | `htui-orch`; `htui` |
| T3 Postgres e2e | htui tests (worktree D) | 2 (§5.4) | `runs_pg` (second test skipped until rebased on T2) |
| merge | — | **T2, then T3** (D90) | T3: full `runs_pg` after the rebase; then the workspace gate (§8) |

---

## 2. T0: the check inside the admission transaction (D76, D77, D80, D81)

**First failing test**: `model::overlap::tests::missing_tags_failure_joins_in_the_given_order`.

**Files** (the plan's list, unchanged): `crates/htui-core/src/model/overlap.rs`,
`crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/store/traits.rs`,
`crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`,
`crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`,
`crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/pg_criteria.rs`,
`crates/htui-store/.sqlx/`.

### 2.1 `overlap.rs`: the variant, the sentence (`Claim`, `:126-169`)

```rust
/// Plan D83: `WriteStore::claim_run`'s verdict. MOD-7 milestone 3 (D80) adds `MissingTags`, the
/// one verdict that writes, and its `Vec` payload is why the type is no longer `Copy`.
#[must_use = "a refused claim must be acted on; only `MissingTags` wrote anything"]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The run is now `running` on the box.
    Admitted,
    /// Not `queued`, or `target_box_id != box`.
    NotClaimable,
    /// `R-ORCH-10` at claim (MOD-7 milestone 3, D80, D81): the run's item requires tags the box
    /// has neither probed nor declared. **The one refusal that writes**: inside the admission
    /// transaction the run moved `queued -> failed` with [`missing_tags_failure`] as its
    /// `failure`, and its item `queued -> blocked`. Decided after `NotClaimable` and before the
    /// slot and the overlap, so a permanent refusal wins over a transient one.
    MissingTags {
        /// The missing tags: byte order, deduplicated, never empty.
        missing: Vec<String>,
    },
    /// The box already runs `limit` runs.
    SlotFull { /* unchanged */ },
    /// The first overlapping live run in `(queued_at, id)` order.
    Overlaps { /* unchanged */ },
}
```

- `is_admitted` (`:150-155`) stays `pub const fn`: a `matches!` on `&self` against a unit variant
  drops nothing and is valid in a `const fn` whatever the other payloads hold.
- `Display` (`:158-169`) gains one arm, after `NotClaimable`:
  `Self::MissingTags { missing } => f.write_str(&missing_tags_failure(missing)),`.
- After `impl fmt::Display for Claim`:

```rust
/// `R-ORCH-10`'s one sentence (MOD-7 milestone 3, D77): `missing tags: a, b`, the tags joined by
/// `", "` **in the order given**. Callers pass what both stores' reads answer (byte order,
/// deduplicated, non-empty). Both stores' `claim_run` write it into `run.failure`, and it is the
/// body of the engine's note at enqueue and at claim (ANA-2 §4.10, §12 criterion 14). It lives
/// here, not in `htui-orch`, because the stores cannot see the engine (the reason the finish-run
/// sentences live in `store::traits`).
#[must_use]
pub fn missing_tags_failure(missing: &[String]) -> String {
    format!("missing tags: {}", missing.join(", "))
}
```

`model/mod.rs:126` becomes
`pub use overlap::{Claim, OverlapRule, RepoScope, RunScope, missing_tags_failure, overlaps, scope_of};`
(rustfmt order).

**Unit tests** (`overlap.rs` `mod tests`, after `claim_display_names_the_rule_and_the_holding_run`):

| Test | Asserts |
|---|---|
| `missing_tags_failure_joins_in_the_given_order` | `missing_tags_failure(&["a".into(), "b".into()]) == "missing tags: a, b"`; `&["a".into()]` → `"missing tags: a"`; `&["vulkan".into(), "docker".into()]` → `"missing tags: vulkan, docker"` (it does not sort: the stores do). |
| `a_missing_tags_claim_displays_the_sentence` | `Claim::MissingTags { missing: vec!["docker".into(), "vulkan".into()] }.to_string() == "missing tags: docker, vulkan"`, equal to `missing_tags_failure` of the same list, and `!is_admitted()`. |

### 2.2 The `Copy` fix (`crates/htui-store/tests/pg_criteria.rs`, `admission_is_serialised_by_the_box_row_lock`)

```rust
    let refusal = if one.is_admitted() { &two } else { &one };           // :694
    assert_eq!(
        refusal,
        &Claim::SlotFull {                                                // :697
            running: 1,
            limit: 1
        },
        "the loser counted the winner's run against the one slot"
    );
    …
    let loser = if one.is_admitted() { second } else { first };          // :709, unchanged
```

It lands in commit (a) with the derive change (H-2). Nothing else in the file changes.

### 2.3 `traits.rs`: the contract (`WriteStore::claim_run` doc, `:821-850`)

- Decision order (`:823-830`): after "[`Claim::NotClaimable`] when the run is not `queued` or its
  `target_box_id` is not `box_id`;", insert: "[`Claim::MissingTags`] when the run's item requires a
  tag in neither the box's `probed_tags` nor its `declared_tags` (`R-ORCH-10`, MOD-7 milestone 3
  D80, D81: exact bytes, the missing tags in byte order and deduplicated; a run with no item has
  none);". The rest of the chain is unchanged.
- The writes paragraph (`:839-841`) becomes: "On [`Claim::Admitted`] the run moves … and the item
  `queued -> in_progress`. On [`Claim::MissingTags`] the run moves `queued -> failed` with
  `failure` = [`missing_tags_failure`](crate::model::missing_tags_failure) of the list and
  `finished_at = at`, and its item `queued -> blocked`, in the same transaction;
  `executing_box_id`, `started_at` and the lease stay unset, so no slot is taken. Every other answer
  writes nothing."

### 2.4 `MemStore` (`mem.rs`)

- Import (the model list at the top): `missing_tags_failure`.
- **D92, one helper for both reads.** On `State`, beside `box_capabilities` (`:3346-3357`):

```rust
    /// `R-ORCH-10`'s set (MOD-7 D76): the entries of `required` in neither `probed_tags` nor
    /// `declared_tags` of `box_id`, sorted by bytes and deduplicated. One helper, so
    /// `MemStore::missing_tags` and `claim_run` cannot order or dedup differently.
    fn missing_for(&self, required: &[String], box_id: BoxId) -> Vec<String> {
        let capabilities = self.box_capabilities(box_id);
        let mut missing: Vec<String> = required
            .iter()
            .filter(|tag| !capabilities.contains(*tag))
            .cloned()
            .collect();
        missing.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        missing.dedup();
        missing
    }
```

  `MemStore::missing_tags` (`:678-696`) keeps its lookups and error order and ends with
  `Ok(state.missing_for(&required, box_id))`. Its doc (`:672-673`, "what the Backlog renders") is
  left as it is (plan OQ-24).
- `State::claim_run` (`:3549-3638`). Doc `:3549` becomes: "ANA-2 §4.7's admission and
  `R-ORCH-10`'s claim-time check (MOD-7 milestone 3, D80), decided before any write: a refusal
  writes nothing, except `MissingTags`, which fails the run and blocks its item." After the
  `NotClaimable` return (`:3566-3568`), before the slot comment (`:3570`):

```rust
        // R-ORCH-10 at claim (MOD-7 milestone 3, D80, D81): after claimability, before the slot
        // and the overlap, so a permanent refusal wins over a transient one. A run with no item
        // (a chat run), or whose item row is gone, has no tags to check, as the Postgres join
        // finds none (D94).
        let missing = claimed
            .item_id
            .and_then(|item| self.items.get(&item))
            .map(|row| self.missing_for(&row.required_tags, box_id))
            .unwrap_or_default();
        if !missing.is_empty() {
            if let Some(row) = self.runs.get_mut(&run) {
                row.status = RunStatus::Failed;
                row.failure = Some(missing_tags_failure(&missing));
                row.finished_at = row.finished_at.or(Some(at));
                row.updated_at = now;
            }
            if let Some(item) = claimed.item_id
                && self
                    .items
                    .get(&item)
                    .is_some_and(|row| row.status == Status::Queued)
            {
                // A stale item status is not a refusal, as in the admitted branch below.
                self.transition(item, Status::Queued, Status::Blocked, now)?;
            }
            return Ok(Claim::MissingTags { missing });
        }
```

  The borrow of `self.items` ends before `get_mut` (the `map` returns an owned `Vec`), so no
  split-borrow is needed. `executing_box_id`, `started_at`, `lease_box_id`, `lease_expires_at` and
  `lease_owners` are untouched. `queued -> blocked` is a legal item move (`item.rs:52-66`) and
  `State::transition` sets `closed_at = None`, which a queued item already has.
- No new `mem.rs` unit test: both stores' behaviour is pinned by the two conformance cases, which
  run over `MemStore` in `tests/mem_store.rs`.

### 2.5 `PgStore` (`pg/write.rs`, `claim_run`, `:2990-3132`)

- Import (`:22-37`): `missing_tags_failure` into the model list.
- Doc `:2986-2989`: "… Every refusal that is not an error is an `Ok` [`Claim`] other than
  [`Claim::Admitted`], with nothing written, except [`Claim::MissingTags`]: the run is failed and its
  item blocked in this same transaction, which is then committed (MOD-7 milestone 3, D80). The run
  row and the box row are both locked `FOR UPDATE` before the tag read, so a concurrent
  `record_box_probe` or `edit_box` waits for the decision and the check cannot race a re-probe."
- After `NotClaimable` (`:3030-3032`), before the limit read (`:3034`), three new statements
  (**`.sqlx` +3**):

```rust
        // R-ORCH-10 at claim (MOD-7 milestone 3, D80, D81): inside the admission transaction,
        // after claimability and before the slot and the overlap. The shipped read's form
        // (`pg/read.rs:1922-1926`), joined through the run; a chat run joins no item and is never
        // refused.
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
```

  The item statement is D80's text exactly. It does not write `closed_at`: a `queued` item's
  `closed_at` is already NULL, because `transition` writes NULL on every move but `done` (`:821-822`)
  and `create_run` only moves `open | failed` (neither terminal). Nothing writes `updated_at` (the
  trigger does). `failed` from `queued` is a legal run move (`run.rs:60-70`). `ck_run_graph_snapshot`
  is satisfied by every `create_run` row (H-1). If `prepare` infers `"tag!"` differently from the
  shipped read, keep the `!` override; it is the same expression.

### 2.6 `.sqlx` (+3, 264 → 267)

```bash
df -h /                                      # before building a second target/ (63 GB free at 5113d00)
docker compose exec -T postgres dropdb -U postgres --if-exists htui_prepare_mod7m3
docker compose exec -T postgres createdb -U postgres htui_prepare_mod7m3
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m3 \
  sqlx migrate run --source crates/htui-store/migrations            # through 0007
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m3 \
  cargo sqlx prepare -- --all-targets --all-features)
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m3 \
  cargo sqlx prepare --check -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l           # 267
git status --porcelain crates/htui-store/.sqlx   # exactly three `??` lines; no `M`, no `D` (H-5)
```

The service is `postgres` (container `htui-postgres`, host port 5439, `compose.yaml:15-24`). The
test DSN (`…/postgres`) and the prepare DSN (`…/htui_prepare_mod7m3`) are different databases
(project memory).

### 2.7 Store conformance cases (`CASES` 74 → 76) and pins

Appended to `CASES` after `"prompt_template_refuses_what_parse_refuses"` (`store/conformance.rs:117`),
with their `run_case` arms before the `other =>` panic, and the two `async fn`s placed after
`claim_run_applies_the_isolation_and_path_rules` (`:4354`), beside the other claim cases. The model
import (`:16-31`) gains `missing_tags_failure`; `BoxEdit` and `CasOutcome` are already imported.
Setup rule (H-1): every run comes from `create_run(new_run(..))`. Items are minted with their tags
(D93): `NewItem { required_tags: tags(&[..]), ..new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT,
"…") }`, with a local `tags(&[&str]) -> Vec<String>`. Every run's `repo_scope` is `Vec::new()`
unless named (hazard H-10: an empty scope never overlaps). `owner = Uuid::now_v7()`,
`at = seam_clock()`, `until = at + TimeDelta::minutes(5)`, as `claim_run_admits_one_and_refuses_the_second`
(`:4158`) does.

| Case | Asserts |
|---|---|
| `claim_run_fails_a_run_whose_item_needs_a_tag_the_box_lacks` | Doc: `R-ORCH-10` at claim (ANA-2 §4.10, MOD-7 milestone 3 D80). The `htui-orch` claim-time case is named in prose only (H-6). (1) `ANA-2`'s run is `Admitted` (one running). (2) Item `x` minted with `["rust", "vulkan", "docker", "vulkan"]` (`rust` probed; `vulkan` doubled to pin the dedup; given out of byte order to pin the sort). Its run `r` is created and `x` is `Queued`. (3) `claim_run(r, ..)` is `Claim::MissingTags { missing: ["docker", "vulkan"] }`. (4) `run_row(r)`: `status == Failed`, `failure == Some(missing_tags_failure(&["docker".into(), "vulkan".into()]))`, and that equals `Some("missing tags: docker, vulkan")`; `finished_at == Some(at)`; `executing_box_id`, `started_at`, `lease_box_id` and `lease_expires_at` all `None`. (5) `item_row(x).status == Blocked`. (6) `claim_run(r, ..)` again is `NotClaimable` and `run_row(r)` is unchanged. (7) `CLEAN-1`'s run is `Admitted`, **the second running run on a box of two**: the failed run took no slot. (8) `transition_run(CLEAN-1's run, Running, AwaitingApproval, at)` frees a slot (a parked run holds none). `transition(x, Blocked, Open)` is `true`. `edit_box(ids::BOX, 0, BoxEdit { declared_tags: Some(tags(&["docker", "gpu", "vulkan"])), quirks: None })` is `CasOutcome::Applied(_)`. A fresh run of `x` is `Admitted`, and `item_row(x).status == InProgress`. |
| `claim_run_checks_tags_after_claimability_and_before_the_slot` | Doc: D81's order, `NotFound`, `NotClaimable`, **`MissingTags`**, `SlotFull`, `Overlaps`. (1) **Claimability first**: `c` (`["vulkan"]`) gets a run `rc`; `finish_run(rc, Cancelled, None, at)`. `claim_run(rc, ..)` is `NotClaimable`; `run_row(rc)` has `status == Cancelled` and `failure == None`; `item_row(c).status == Open`, not `Blocked`: nothing was written. (2) **Before the overlap** (D93): `repo = create_repo(new_repo(PROJECT_HTUI, "core", true))`. `ANA-2`'s run with `[repo]` is `Admitted`. `a` (`["vulkan"]`) with `[repo]` answers `MissingTags { missing: ["vulkan"] }`, **not** `Overlaps { with: ANA-2's run, .. }`, and `a` is `Blocked`. (3) **Before the slot**: `CLEAN-1`'s run (no scope) is `Admitted`, filling the box (2 of 2). `b` (`["docker"]`) answers `MissingTags { missing: ["docker"] }`, **not** `SlotFull`. (4) Control: an untagged minted item's run answers `SlotFull { running: 2, limit: 2 }`, so the box really was full. |

**Pins**: `crates/htui-core/tests/mem_store.rs:37` becomes `76`, and the message ends "…, MOD-9
milestone 1's three for the template writer (plan D1-D4), and MOD-7 milestone 3's two for the
claim-time capability check (plan D80, D81)". `crates/htui-store/tests/pg_conformance.rs:19` becomes
`const EXPECTED_CASES: usize = 76;`. `READ_CASES` stays 14.

### 2.8 Commits (T0) and gate

1. **(a) red**: `overlap.rs` (variant, `Copy` dropped, `#[must_use]` text, `Display` arm,
   `missing_tags_failure` with a `todo!()` body, the two unit tests); `model/mod.rs`;
   `pg_criteria.rs`'s two-line fix (H-2); the two conformance cases, their arms and both pins.
   Compiles offline: `PgStore` has no new query yet. Red: the unit tests hit `todo!()`, and both new
   cases fail on `MemStore` (it answers `Admitted`).
2. **(b) green, `htui-core`**: the `missing_tags_failure` body; `State::missing_for`; the
   `claim_run` check; the `traits.rs` and `mem.rs` docs.
3. **(c) green, `htui-store`**: `PgStore::claim_run`'s check and doc; `.sqlx` (§2.6).

```bash
cargo test -p htui-core --all-features -- --test-threads=1
cargo check --workspace --all-features --all-targets                 # H-2: every crate, every target
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=1
cargo test -p htui-orch --all-features -- --test-threads=1          # base engine unaffected
cargo clippy -p htui-core -p htui-store --all-features --all-targets -- -D warnings
# §2.6: prepare --check, 267 files, three `??` only
```

---

## 3. T1: the seam, the enqueue refusal and criterion 14 (D75, D78, D79, D82, D83, D84, D86)

**First failing test**: `conformance::a_capability_refusal_writes_no_run_and_unblock_reopens`
(through `every_case_name_dispatches`). T1 builds on the **base** `htui-core`: it names nothing T0
adds (no `missing_tags_failure`, no `Claim::MissingTags`) and spells the sentence as a literal in
both places.

**Files** (the plan's list, unchanged): `crates/htui-orch/src/graph.rs`,
`crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/status.rs`,
`crates/htui-orch/src/command.rs`, `crates/htui-orch/src/engine.rs`,
`crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs`,
`crates/htui/src/run_worker.rs`.

### 3.1 `graph.rs`: the seventh method (`GraphSource`, `:57-119`)

Appended after `bound_skills` (`:113-118`):

```rust
    /// `R-ORCH-10`'s read (MOD-7 milestone 3, D75): the entries of `item.required_tags` that
    /// `box_id` has neither probed nor declared (`box.probed_tags ∪ box.declared_tags`), compared
    /// as exact bytes, sorted by bytes and deduplicated. Empty when the box can run the item, and
    /// for an item that requires nothing. Inherent on both stores and on `Backend` (`mem.rs:678`,
    /// `pg/read.rs:1919`, `backend.rs:561`) for `prompt_template`'s reason: `box` is mirrored,
    /// but the mirror runs no orchestration reads.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown item or box, the item first; the backend's own
    /// failures; `Backend` offline refuses with its orchestration sentence.
    async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>>;
```

Trait doc: `:49`'s "`app_setting` is deliberately **not** a fifth method (blueprint A-2)" becomes
"… **not** a method of this trait (blueprint A-2)", and the first paragraph (`:33-47`) gains one
sentence: "MOD-9 added `bound_skills` and MOD-7 milestone 3 `missing_tags`, for the same reason."
`StoreError` is already imported (`:21`), so the link resolves.

`TestSource` (`impl GraphSource for TestSource<'_>`, `:797-846`), after `bound_skills`:
`async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
self.store.missing_tags(item, box_id).await }` (inherent on `&MemStore`).

### 3.2 `fake.rs`

- `impl GraphSource for MemStore` (`:844-892`), after `bound_skills`:
  `async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
  self.missing_tags(item, box_id).await }`. Inherent candidates win over trait ones, so this is
  delegation; the unit test below proves it. Its doc (`:830`, `:843`): "six" → "seven" (P-4).
- `impl GraphSource for FakeGraphSource<'_>` (`:963-…`), after `bound_skills`:
  `GraphSource::missing_tags(self.store, item, box_id).await`, in the shape of its `agent`.
- `the_store_answers_the_source_without_recursing` (`:2117-2173`): doc "six" → "seven". Append
  "MOD-7 milestone 3 D75: the seventh read." It mints `needs_cuda` (`NewItem` with
  `required_tags: vec!["cuda".to_owned()]`, the shape of `mem.rs:8928-8941`; import
  `htui_core::store::WriteStore as _` and `NewItem` in the test if absent). It asserts
  `GraphSource::missing_tags(&store, needs_cuda, ids::BOX)` is `["cuda"]` and equals the inherent
  answer, and that `GraphSource::missing_tags(&store, ids::HTUI_FEAT_3, ids::BOX)` is empty (`rust`
  is probed).

### 3.3 `run_worker.rs` (`BackendGraphs`, `:2364-2403`)

After `bound_skills`:

```rust
    async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> StoreResult<Vec<String>> {
        self.0.missing_tags(item, box_id).await
    }
```

No other `crates/htui` source changes (D84): a `MissingTags` refusal takes the worker's generic
`Some(Err(err)) => ctx.refuse(err.to_string())` / `Err(err) => return ctx.refuse(..)` arms.

### 3.4 `status.rs` (`RunFailure`, `:39-100`; `Display`, `:102-136`)

Last variant:

```rust
    /// `R-ORCH-10` (MOD-7 milestone 3, D83): the item requires tags this box has neither probed
    /// nor declared. Carries them in byte order, deduplicated, as `GraphSource::missing_tags` and
    /// `Claim::MissingTags` answer. The note at enqueue and at claim, and `run.failure` at claim,
    /// read the same bytes (D78).
    MissingTags(Vec<String>),
```

`Display` arm (T1, literal): `Self::MissingTags(missing) => write!(f, "missing tags: {}",
missing.join(", ")),`. `run_failure_display_is_ana2s_bytes` (`:405`) gains
`RunFailure::MissingTags(vec!["a".into(), "b".into()]).to_string() == "missing tags: a, b"` and
`RunFailure::MissingTags(vec!["docker".into(), "vulkan".into()]).to_string() == "missing tags:
docker, vulkan"`.

### 3.5 `command.rs` (`EngineError`, `:310-…`)

Directly after `ClaimRefused` (`:375-383`):

```rust
    /// `R-ORCH-10` (MOD-7 milestone 3, D84): the item requires tags this box has neither probed
    /// nor declared. At `StartRun`'s enqueue (`run: None`) no run row was written and the item is
    /// `blocked` with a note; at the claim (`run: Some`) `claim_run` failed the run inside the
    /// admission transaction and the engine noted it. Permanent until the box gains the tags or
    /// the item drops them, so it is **not** [`EngineError::ClaimRefused`] and nothing re-queues
    /// it.
    #[error("item {item}: missing tags: {}", .missing.join(", "))]
    MissingTags {
        /// The item refused.
        item: ItemId,
        /// The run `claim_run` failed; `None` at enqueue, where no run exists.
        run: Option<RunId>,
        /// The missing tags: byte order, deduplicated, never empty.
        missing: Vec<String>,
    },
```

`the_named_refusals_say_what_went_wrong` (fn `:1614`) gains two rows, after the `ClaimRefused` pair:
`EngineError::MissingTags { item: ids::HTUI_FEAT_3, run: None, missing: vec!["docker".into(),
"vulkan".into()] }` → `format!("item {}: missing tags: docker, vulkan", ids::HTUI_FEAT_3)`. The same
shape with `run: Some(ids::RUN_2)` gives the same bytes: the run is not in the sentence.

### 3.6 `engine.rs`: the enqueue check (`Engine::enqueue`, `:586-641`)

Doc `# Errors` (`:594-595`): "[`EngineError::Resolve`] (rung 4, after its note on the item),
[`EngineError::MissingTags`] (`R-ORCH-10`, after its note on the item, before resolution), and the
store's refusals." The summary line (`:586-589`) adds "refuse missing tags, then rung 4, onto the
item". After `let item = self.item(item).await?;` (`:602`):

```rust
        // R-ORCH-10 at queue time (MOD-7 milestone 3, D79): before resolution, so an item that
        // lacks tags *and* has no candidate is refused for the tags, the cheaper box-level fact.
        // Only an item `create_run` would accept (`open | failed`) is checked; any other status
        // falls through to that refusal and writes no note (D98).
        if matches!(item.status, Status::Open | Status::Failed) {
            let missing = self
                .parts
                .graphs
                .missing_tags(item.id, self.parts.box_id)
                .await?;
            if !missing.is_empty() {
                return Err(self.refuse_missing_tags(&item, missing).await?);
            }
        }
```

New private method, beside `refuse_rung_four` (`:685-706`):

```rust
    /// `R-ORCH-10` at `StartRun` (MOD-7 milestone 3, D79, D82): no run row exists, so the refusal
    /// is written to the item, `open -> blocked` and then `missing tags: a, b` as a note on this
    /// box, and handed back. A `failed` item has no `blocked` edge and the compare-and-set answers
    /// `Ok(false)`, which leaves it `failed` with the note still written (rung 4's rule; invariant
    /// 7).
    async fn refuse_missing_tags(
        &self,
        item: &Item,
        missing: Vec<String>,
    ) -> Result<EngineError, EngineError> {
        self.parts
            .store
            .transition(item.id, Status::Open, Status::Blocked)
            .await?;
        self.note(
            item.id,
            RunFailure::MissingTags(missing.clone()).to_string(),
            None,
            self.now(),
        )
        .await?;
        Ok(EngineError::MissingTags {
            item: item.id,
            run: None,
            missing,
        })
    }
```

**Engine unit tests** (`mod tests`, beside `enqueue_then_claim_is_start_run`, `:11318`, using
`harness_engine!`). A local helper `require_tags(&Harness, ItemId, &[&str])` reads the item and calls
`harness.orch.store.update_item(item, row.version, ItemPatch { required_tags: Some(..),
author_id: row.created_by, reason: "a test's required tags".into(), ..ItemPatch::default() })`.

| Test | Asserts |
|---|---|
| `enqueue_refuses_missing_tags_before_resolving` | `free_feat_3`; `harness.orch.without_candidates("prd")` (`fake.rs:1397`, so rung 4 *would* refuse); `FEAT-3` requires `["vulkan", "rust"]`. `engine.enqueue(FEAT-3, Manual, None)` is `Err(EngineError::MissingTags { item: FEAT-3, run: None, missing: ["vulkan"] })`, not `Resolve(NoCandidate)`. `harness.orch.store.runs(FEAT-3)` is unchanged; the item is `Blocked`; the notes gained exactly `"missing tags: vulkan"`, and none says ``no_candidate_agent: phase `prd` ``. |
| `enqueue_checks_tags_only_where_create_run_would` (D98) | `FEAT-3` is **not** freed (seeded `queued` under `RUN_2`) and requires `["vulkan"]`. `engine.enqueue` is `Err(EngineError::Store(_))`, `create_run`'s refusal and not `MissingTags`; the item stays `Queued`, and its note count is unchanged. This pins H-7: `TOOL-1` and Vulkan `FEAT-1` stay as they were. |

### 3.7 `conformance.rs`: criterion 14's capability half (D86), and the count

Imports: `htui_core::model::BoxEdit`; `htui_core::store::{CasOutcome, MemStore, ReadStore as _,
WriteStore as _}`.

**Helpers**, after `touch` (`:3488-3503`), in its shape:
- `async fn require_tags<O: Orchestrate>(orch: &O, item: ItemId, tags: &[&str])`: `update_item`
  with `required_tags: Some(..)`, reason `"a conformance case's required tags"` (D97; T2 reuses it).
- `async fn declare_tags<O: Orchestrate>(orch: &O, tags: &[&str])`: reads the current token from
  `orch.store().boxes()` (the `ids::BOX` record's `row.edit_version`), then `edit_box(ids::BOX,
  token, BoxEdit { declared_tags: Some(..), quirks: None })`, asserting `CasOutcome::Applied(_)`.

**The case**, placed after `no_candidate_agent_blocks_the_item` (`:2574-2620`):

```rust
/// ANA-2 §12 criterion 14 (`docs/ANA-2.md:2121-2122`), its capability half (MOD-7 milestone 3,
/// D86): an item requiring tags the box has neither probed nor declared is refused at `StartRun`
/// with no `run` row, the item `blocked` and one note whose body is exactly the missing tags,
/// written on this box; `Unblock` reopens it, and once the box declares the tags `StartRun` walks.
async fn a_capability_refusal_writes_no_run_and_unblock_reopens<H: CaseHarness>(harness: &H)
```

Body, in order: `free_feat_3`. `require_tags(FEAT-3, ["rust", "vulkan", "docker"])`. Read
`runs_before = store().runs(FEAT-3).len()` and `notes_before = store().notes(FEAT-3)` (rows).
Dispatch `StartRun` and `expect_err`: `matches!(&refused, EngineError::MissingTags { item, run:
None, missing } if *item == ids::HTUI_FEAT_3 && missing == &["docker", "vulkan"])`, and
`refused.to_string() == format!("item {}: missing tags: docker, vulkan", ids::HTUI_FEAT_3)`.
`runs(FEAT-3).len() == runs_before`. `item_of(FEAT-3).status == Blocked`. The new notes are exactly
one row, with `body == "missing tags: docker, vulkan"`, `box_id == Some(ids::BOX)` and
`via_step_id == None`. `unblock(&orch, FEAT-3)` is `(UnblockCase::Reopen, None)`, and the item is
`Open`. `declare_tags(["docker", "gpu", "vulkan"])`. `start(&orch, FEAT-3).1.run ==
RunStatus::AwaitingApproval`.

**`CASES`**: the entry goes directly after `"no_candidate_agent_blocks_the_item",` (`:384`), with the
comment `// ANA-2 §12 criterion 14 (\`:2121-2122\`, MOD-7 milestone 3 D86): a capability refusal
writes no run, blocks the item with a note naming exactly the missing tags, and \`Unblock\` reopens
it.` Its arm goes in `fn case` (`:513`) after the `no_candidate_agent_blocks_the_item` arm:
`"a_capability_refusal_writes_no_run_and_unblock_reopens" => { Box::pin(a_capability_refusal_writes_no_run_and_unblock_reopens(harness)) }`.

**The MOD-4 case, reworded in three places** (P-3). The case name and body are unchanged.
- Fn doc (`:5197-5199`): "Rung 4's `Unblock` round trip (MOD-4 plan D161 case 1): rung 4 at
  `StartRun` leaves the item `blocked` with no run; `Unblock` reopens it, and once the phase has a
  candidate again `StartRun` walks it. Criterion 14 itself is
  `a_capability_refusal_writes_no_run_and_unblock_reopens` (MOD-7 milestone 3, D86)."
- `CASES` comment (`:481`): `// Rung 4's \`Unblock\` round trip (plan D161): its blocked item
  reopens. Criterion 14 itself is MOD-7 milestone 3's case above.`
- Preamble (`:330-332`): "`Unblock`'s three cases (rung 4's reopen, R-4's escalated run, R-7's
  refused reconcile)".

**Count checklist (H-4, D96)**, T1's values:
- `:288`: "Seventy, and the count is pinned …" → "Seventy-one, …".
- `:289`: `` `cases_are_unique_and_seventy` `` → `` `cases_are_unique_and_counted` ``.
- `:291`: "Recounted, not appended: 18 + 5 + 13 + 6 + 10 + 18." → "… 18 + 5 + 13 + 6 + 10 + 18 + 1."
- A new paragraph after the milestone 6 paragraph (before `pub const CASES`): "**One for MOD-7
  milestone 3** (plan D86): criterion 14's capability half, a refusal at `StartRun` that writes no
  run and that `Unblock` reopens."
- `:507`: "a seventy-arm `match` … puts all seventy in the one frame" → "a seventy-one-arm `match`
  … puts all seventy-one …".
- Pin test (`:5510`): rename to `cases_are_unique_and_counted`, `70` → `71`, the sum → "18 + 5 + 13
  + 6 + 10 + 18 + 1", and the message ends "…, and criterion 20's close-out and its refusal), and
  MOD-7 milestone 3's one (criterion 14's capability half)".
- `tests/fake_conformance.rs:15-17`: `fn cases_len_is_pinned() { assert_eq!(CASES.len(), 71); }`.

### 3.8 Build coupling

T1 consumes the base's inherent `MemStore::missing_tags` and `Backend::missing_tags` and the
existing `WriteStore::{update_item, edit_box, boxes}`. It builds against the base `htui-core`; T0's
merge adds `Claim::MissingTags`, which T1 never matches (its engine still calls `is_admitted()` and
moves the verdict into `ClaimRefused`).

### 3.9 Commits (T1) and gate

1. **(a) red**: the trait method and all four implementations (pure delegation, green on arrival);
   the `fake.rs` doc and test changes; `RunFailure::MissingTags` with its literal `Display` and byte
   rows; `EngineError::MissingTags` with its sentence rows; the two engine unit tests; the
   conformance helpers, case, `CASES` entry, `fn case` arm; both pins and the prose (§3.7). Red: the
   case and the first engine test fail (the engine does not check yet).
2. **(b) green**: the enqueue check, `refuse_missing_tags`, the `enqueue` doc, and the three MOD-4
   rewordings.

```bash
cargo test -p htui-orch --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features -- --test-threads=1      # runs_pg starts FEAT-3 (`rust`) through BackendGraphs
cargo clippy -p htui-orch -p htui --all-features --all-targets -- -D warnings
rg -n 'eventy' crates/htui-orch                                    # only "Seventy-one" / "seventy-one"
rg -n 'cases_are_unique_and_seventy|cases_len_is_seventy' crates   # nothing
```

---

## 4. T2: the engine side of the claim (D83, D84, D85, D87)

**First failing test**: `engine::tests::a_missing_tags_claim_is_not_a_claim_refused`. T2 starts from
T0 and T1 merged (worktree C).

**Files** (the plan's list): `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/command.rs`,
`crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/conformance.rs`,
`crates/htui-orch/tests/fake_conformance.rs`.

### 4.1 One sentence (D77, D83, D84)

- `status.rs`: the arm becomes
  `Self::MissingTags(missing) => f.write_str(&missing_tags_failure(missing)),` with
  `use htui_core::model::missing_tags_failure;` beside the model import (`:10`).
- `command.rs`: `#[error("item {item}: {}", htui_core::model::missing_tags_failure(.missing))]`.
  thiserror hands `.missing` in as `&Vec<String>`, which coerces to `&[String]` at the call.
- T1's byte rows in `run_failure_display_is_ana2s_bytes` and
  `the_named_refusals_say_what_went_wrong` stay as they are and now pass through `htui_core`'s fn.
  That the bytes do not move is the test.

### 4.2 `ClaimRefused`'s doc (`command.rs:375-383`)

"What `claim_run` answered; never [`Claim::Admitted`]." becomes "… never [`Claim::Admitted`], and
never [`Claim::MissingTags`], which is [`EngineError::MissingTags`] (MOD-7 milestone 3, D85)." The
variant doc keeps "The run stays `queued`", which is now true of every `ClaimRefused`.

### 4.3 `Engine::claim` (`engine.rs:654-683`)

Import `htui_core::model::Claim` (the tests name it fully qualified today). Doc `# Errors`:
"[`EngineError::ClaimRefused`] naming the rule when the box is full or the scope overlaps, the run
then left `queued` with nothing written; [`EngineError::MissingTags`] when the run's item needs tags
this box lacks, after `claim_run` failed the run and blocked its item in its transaction and this
engine noted it (`R-ORCH-10`, MOD-7 milestone 3 D85); …". The comment `:669-671` becomes: "Anything
but `Admitted` is a refusal. `MissingTags` is `R-ORCH-10`'s: `claim_run` already failed the run and
blocked its item, and the engine adds the note (D78). Every other verdict is ANA-2 §4.7's
admission, box full or overlap (plan D83), which wrote nothing and left the run `queued`." Body:

```rust
        let claim = self
            .parts
            .store
            .claim_run(run, self.parts.box_id, self.parts.owner, now, lease)
            .await?;
        match claim {
            Claim::Admitted => {}
            // Matched before the catch-all, so it can never reach the worker as `ClaimRefused`
            // and be re-queued (`htui/src/run_worker.rs`'s two re-queue arms, H-3).
            Claim::MissingTags { missing } => {
                let item = Self::item_of(&self.run(run).await?)?;
                self.note(
                    item,
                    RunFailure::MissingTags(missing.clone()).to_string(),
                    None,
                    now,
                )
                .await?;
                return Err(EngineError::MissingTags {
                    item,
                    run: Some(run),
                    missing,
                });
            }
            claim => return Err(EngineError::ClaimRefused { run, claim }),
        }
```

`Engine::run` (`:5456`) and `Engine::item_of` (`:5364`) exist. The store never answers `MissingTags`
for a chat run (it has no item; D94), so `item_of`'s `RunStatus` refusal is unreachable by contract
and needs no new sentence.

### 4.4 Engine unit test

| Test | Asserts |
|---|---|
| `a_missing_tags_claim_is_not_a_claim_refused` | `free_feat_3`; `FEAT-3` requires `["gpu", "rust"]` (both present on the fixture box, so the enqueue passes); `harness_engine!`. `engine.enqueue(..)` answers `run`. `harness.orch.store.edit_box(ids::BOX, 0, BoxEdit { declared_tags: Some(vec![]), quirks: None })` is `Applied`: the re-probe between the two. `engine.claim(run)` is `Err(e)` with `matches!(&e, EngineError::MissingTags { item, run: Some(r), missing } if *item == FEAT-3 && *r == run && missing == &["gpu"])` and `!matches!(e, EngineError::ClaimRefused { .. })`; `e.to_string() == format!("item {}: missing tags: gpu", FEAT-3)`. The run is `Failed`, with `failure == Some("missing tags: gpu")` and `started_at == None`; the item is `Blocked`; the notes gained exactly `"missing tags: gpu"`. The doc says the worker's not-re-queued half is T3's (plan amendment). |

### 4.5 `conformance.rs`: the claim-time case (D87, P-5)

```rust
/// ANA-2 §4.10's claim-time half (MOD-7 milestone 3, D87; the PRD's "tag check at claim races a
/// re-probe" risk): a run queued behind an overlap while its tag was declared, whose tag is then
/// withdrawn, is failed at its next claim with `missing tags: gpu`, and the check comes before
/// the overlap that still stands. The item is `blocked` with the note, and `Unblock` reopens it.
async fn a_capability_refusal_at_claim_fails_the_run_by_name<H: CaseHarness>(harness: &H)
```

Body: `primary_repo(&orch).await` (P-5). `first = mint_feat(&orch, "Holder", &["src/**"])` and
`second = mint_feat(&orch, "Needs a GPU", &["src/**"])`. `require_tags(second, ["gpu"])` (declared on
the fixture box). `let (holder, rest) = start(&orch, first)`, and `rest.run == AwaitingApproval` (a
parked run still holds its scope). `let (run, claim) = start_refused(&orch, second)`: `claim ==
Claim::Overlaps { with: holder, rule: OverlapRule::Paths }`, and `run_of(run).status == Queued`.
`declare_tags(&orch, &[])`: the re-probe. `orch.claim(run)` is
`Err(EngineError::MissingTags { item: second, run: Some(run), missing: ["gpu"] })`, **even though
`holder` still overlaps** (D81). `run_of(run)`: `status == Failed`, `failure == Some("missing tags:
gpu")`, `executing_box_id`, `started_at` and `lease_expires_at` all `None`, `finished_at.is_some()`.
`steps_of(run)` is empty. `item_of(second).status == Blocked`. Exactly one new note on `second`:
`"missing tags: gpu"`, `box_id == Some(ids::BOX)`. `run_of(holder).status == AwaitingApproval`
(untouched). `unblock(&orch, second) == (UnblockCase::Reopen, None)`: the failed run is not active.

`CASES`: after T1's entry, `// ANA-2 §4.10's claim-time half (MOD-7 milestone 3 D87): a tag
withdrawn between enqueue and claim fails the run by name, before the overlap.` Plus its `fn case`
arm after T1's.

**Count checklist (H-4)**, T2's values: "Seventy-two"; the sum "18 + 5 + 13 + 6 + 10 + 18 + 2";
T1's paragraph becomes "**Two for MOD-7 milestone 3** (plan D86, D87): criterion 14's capability
half, … and ANA-2 §4.10's claim-time half, a run failed by name when its tag was withdrawn after
enqueue"; "seventy-two-arm" and "all seventy-two"; pin `72` with the message's last clause "MOD-7
milestone 3's two (criterion 14's capability half and §4.10's claim-time half)";
`tests/fake_conformance.rs` → `72`.

### 4.6 Build coupling

T2 needs T0's `Claim::MissingTags` and `missing_tags_failure`, and T1's `EngineError::MissingTags`,
`RunFailure::MissingTags`, `require_tags` and `declare_tags`. It touches no `crates/htui` file.

### 4.7 Commits (T2) and gate

1. **(a) refactor, green**: §4.1 (both sentences through `htui_core`) and §4.2. The bytes do not
   move; T1's rows prove it.
2. **(b) red**: the engine unit test; the §4.5 case, its entry and arm; the count checklist. Red:
   the engine still answers `ClaimRefused { claim: MissingTags }`.
3. **(c) green**: §4.3.

```bash
cargo test -p htui-orch --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features -- --test-threads=1
cargo clippy -p htui-orch --all-features --all-targets -- -D warnings
rg -n 'eventy' crates/htui-orch        # only "Seventy-two" / "seventy-two"
```

---

## 5. T3: Postgres end to end (D89, D90, D91, D99, D100)

**Files**: `crates/htui/tests/runs_pg.rs` only. Runtime `sqlx::query*` only; `crates/htui` has no
`.sqlx/`. T3 starts from T0 and T1 merged (worktree D) and **merges after T2** (D90).

### 5.1 Additions to the file

- Module doc (`:1-24`): one paragraph. "MOD-7 milestone 3 adds criterion 14's capability half and
  ANA-2 §4.10's claim-time half, driven through the same `RunRuntime`: the enqueue refusal reads
  tags through `run_worker::BackendGraphs`, and the claim's through `PgStore::claim_run`."
- Imports: `htui_core::model::{BoxEdit, ItemPatch}`; `htui_core::store::CasOutcome`.
- Free fns beside `free_feat_3` (`:504`):
  - `async fn require_tags(store: &PgStore, item: ItemId, tags: &[&str])`: `update_item` with the
    row's `version`, `required_tags: Some(..)`, `author_id: row.created_by`.
  - `async fn declare_tags(store: &PgStore, expected: i32, tags: &[&str])`: `edit_box(ids::BOX,
    expected, BoxEdit { declared_tags: Some(..), quirks: None })`, asserting
    `CasOutcome::Applied(_)`. The demo box starts at `edit_version` 0 on Postgres (the loader never
    names the column).
  - `async fn note_box(pool: &PgPool, item: ItemId, body: &str) -> Vec<Option<uuid::Uuid>>`:
    `SELECT box_id FROM item_note WHERE item_id = $1 AND body = $2 ORDER BY created_at`.

### 5.2 `a_capability_refusal_blocks_and_unblock_reopens_on_postgres` (criterion 14 in full)

`Stack::new(None)`; `item = ids::HTUI_FEAT_3`; `free_feat_3`; `require_tags(item, ["rust",
"vulkan", "docker"])`. `assert_eq!(stack.run_ids(item).await, vec![ids::RUN_2])`: the seeded
cancelled run. `stack.command(StartRun { item, mode: Manual, repo_scope: None })`.
`stack.take_status() == Some(format!("start_run: item {item}: missing tags: docker, vulkan"))`
(P-6). `run_ids(item) == [ids::RUN_2]`: no run row. `stack.item(item).status == Blocked`.
`note_box(pool, item, "missing tags: docker, vulkan") == [Some(ids::BOX.as_uuid())]`: one note, on
this box. `stack.command(Unblock { item })`; `take_status() == None`; the item is `Open`; the notes
contain ``"unblocked: back to `open`"``. `declare_tags(0, ["docker", "gpu", "vulkan"])`.
`let run = stack.start(item).await` (it asserts no refusal and one new run), and
`stack.run(run).status == AwaitingApproval`. `stack.finish()`.

### 5.3 `a_claim_time_refusal_is_not_requeued_on_postgres` (D91, D99)

`Stack::new(None)`; `free_feat_3`; `require_tags(FEAT-3, ["rust", "gpu"])`.
1. `holder = stack.start(ids::HTUI_ANA_2)`, which parks. `seed`'s primary repo is the scope both
   undeclared items hold (P-7).
2. `stack.command(StartRun { FEAT-3 })`. `take_status()` starts with
   `format!("start_run: claim refused: overlaps run {holder} (")` (the rule is not pinned here; the
   `htui-orch` cases pin it). The one new run id is `refused`; `stack.run(refused).status == Queued`
   and the item is `Queued`.
3. `declare_tags(0, [])`: the re-probe.
4. `stack.command(CancelRun { run: holder })`; `take_status() == None`. Ending that task runs
   `retry_claims` → `claim_queued` → `reclaim(refused)`.
5. `stack.drive_until("the retried claim's note", |store| async move { store.notes(FEAT-3).await
   .is_ok_and(|n| n.iter().any(|note| note.body == "missing tags: gpu")) })`. Before T2 this times
   out (P-2).
6. `stack.run(refused)`: `status == Failed`, `failure == Some("missing tags: gpu")`,
   `executing_box_id == None`, `started_at == None`. The item is `Blocked`.
   `note_box(pool, FEAT-3, "missing tags: gpu")` has exactly one entry, `Some(ids::BOX)`.
7. `take_status() == None`: a claim retry answers nobody (P-1). The note is the refusal a human
   reads, and it exists only because `Engine::claim` answered `EngineError::MissingTags`, the
   variant neither re-queue arm matches (H-3).
8. `Unblock { FEAT-3 }` → the item is `Open`. `declare_tags(1, ["gpu"])`.
   `stack.start(FEAT-3)` walks and parks: no holder overlaps now.

### 5.4 Commits (T3) and gate

1. **(a)** §5.1 helpers and §5.2. Green on the Wave-1 tree.
2. **(b)** §5.3. Red in T3's worktree until T2 is merged (P-2).

```bash
# in worktree D, before the rebase:
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features --test runs_pg -- --test-threads=1 \
  --skip a_claim_time_refusal_is_not_requeued_on_postgres
# after T2 is merged and T3 rebased onto it:
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features --test runs_pg -- --test-threads=1
```

A red §5.2 on the Wave-1 tree is a T0 or T1 defect. A red §5.3 after the rebase is a T0 or T2
defect. Either is routed back rather than patched from T3.

---

## 6. Cross-task contracts

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T0 `htui_core::model` | `Claim::MissingTags { missing: Vec<String> }`; `Claim: Debug + Clone + PartialEq + Eq` (no `Copy`); `Display` = the sentence | T2 (engine), T3 (through the worker) |
| T0 `htui_core::model` | `pub fn missing_tags_failure(missing: &[String]) -> String` = `format!("missing tags: {}", missing.join(", "))` | T0 stores, T2 (`RunFailure`, `EngineError`) |
| T0 `WriteStore::claim_run` | order `NotFound(run)`, `NotFound(box)`, `NotClaimable`, **`MissingTags`**, `SlotFull`, `Overlaps`; on `MissingTags`: run `queued -> failed`, `failure` = sentence, `finished_at = COALESCE(.., at)`, item `queued -> blocked`, one transaction | T2, T3 |
| T1 `htui_orch::graph::GraphSource` | `async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>>` | T1 engine, T3 (`BackendGraphs`) |
| T1 `htui_orch::status` | `RunFailure::MissingTags(Vec<String>)`, `Display` = the sentence | T1 and T2 notes |
| T1 `htui_orch::command` | `EngineError::MissingTags { item: ItemId, run: Option<RunId>, missing: Vec<String> }`, `Display` = `item {item}: missing tags: a, b` | T2, T3 (status line `start_run: item …`) |
| T1 `htui_orch::conformance` (private) | `require_tags`, `declare_tags` | T2 |

**Byte-exact strings**: `missing tags: docker, vulkan` (note body, `run.failure`,
`RunFailure`/`Claim` `Display`); `item <uuid>: missing tags: docker, vulkan` (`EngineError`);
`start_run: item <uuid>: missing tags: docker, vulkan` (the status line, P-6).

**Parallel-lane hazards**: T0 ∩ T1 = ∅ and T2 ∩ T3 = ∅ (the plan's check, re-verified). `.sqlx`
moves in T0 only. Store pins move in T0 only; `htui-orch` pins in T1 and T2 (serial). No snapshot or
fixture moves anywhere. Two worktrees mean two `target/` directories: check `df -h /` (63 GB free)
before each wave.

---

## 7. Count pins

| Pin | Now | After | Task |
|---|---|---|---|
| Store `CASES` | 74 | 76 | T0 (`store/conformance.rs:43-118`, `mem_store.rs:37`, `pg_conformance.rs:19`) |
| `READ_CASES` | 14 | 14 | — |
| `htui-orch` `CASES` | 70 | 71 → 72 | T1, T2 (`conformance.rs` pin test, `tests/fake_conformance.rs`) |
| `GraphSource` methods | 6 | 7 | T1 |
| `.sqlx` files | 264 | 267 (three `??`, nothing modified) | T0 |
| `StoreRequest` / `StoreReply` | 68 / 39 | unchanged | — |
| Migrations | `0001`..`0007` | unchanged; next `0008` | — |
| `crates/htui/tests/snapshots` | 87 | 87 | — |

---

## 8. Merge order and the workspace gate

[T0 ∥ T1] → merge T0 (core, store, orch gates) → merge T1 (orch, htui gates) → [T2 ∥ T3] → merge T2
(orch, htui gates) → T3 rebases, runs its full gate → merge T3 → workspace gate → optional live check.

```bash
df -h /
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
docker compose exec -T postgres dropdb -U postgres --if-exists htui_prepare_mod7m3
docker compose exec -T postgres createdb -U postgres htui_prepare_mod7m3
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m3 \
  sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m3 \
  cargo sqlx prepare --check -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l                    # 267
cargo doc --workspace --no-deps --keep-going          # exactly the five baseline errors (HANDOFF.md:42-45)
git diff --stat 98e6d2f -- crates/htui-store/migrations crates/htui-store/cache_migrations \
  crates/htui/tests/snapshots crates/htui-core/src/fixtures.rs    # empty
rg -n 'eventy' crates/htui-orch                       # "Seventy-two" / "seventy-two" only
```

Before believing a Postgres failure, run `df -h /` and re-run the case alone (project memory: the
dev Postgres also restarts under load and reports "healthy" while still recovering).

---

## 9. Decisions (D90 onward) and risks (R-41 onward)

| # | Decision |
|---|---|
| D90 | Wave 2 keeps T2 ∥ T3 for authoring, but **T2 merges first**. T3 gates its second test only after rebasing onto T2, and skips it by name before that (P-2). |
| D91 | T3's claim-time test asserts the **note** as the proof that the engine answered `MissingTags` and not `ClaimRefused`, and asserts that the status line stays `None`, because `reclaim` is unaddressed (P-1). The not-re-queued property is structural: `run_worker.rs`'s re-queue arms match `ClaimRefused { .. }` only, and the reviewer checks them. |
| D92 | `MemStore` gets one `State::missing_for(required, box_id)` shared by `MemStore::missing_tags` and `claim_run`, so the sort and dedup cannot drift between the read and the check. |
| D93 | The store cases mint their items with the tags (`NewItem { required_tags, ..new_item(..) }`) instead of `update_item`: the same row, without the version dance. They also pin dedup (a doubled tag) and `MissingTags` before `Overlaps` in the store, which the plan's store cases did not reach. |
| D94 | A run with no item, or whose item row is absent, is never refused: `MemStore` uses `items.get` (not `require_item`), mirroring the Postgres join that finds no row. |
| D95 | `Engine::claim` reads the item with the existing `Self::item_of(&self.run(run).await?)?`. The note body is `RunFailure::MissingTags(..).to_string()`, the same value the enqueue note uses. |
| D96 | The `htui-orch` pin tests are renamed `cases_are_unique_and_counted` and `cases_len_is_pinned` (T1), per the plan's fact-check amendment. The count prose follows §3.7's checklist, and T2 re-applies it with its values. |
| D97 | T1 adds the conformance helpers `require_tags` and `declare_tags`; `declare_tags` reads the live `edit_version` rather than assuming 0. T2 reuses both. |
| D98 | T1 pins D79's status guard with `enqueue_checks_tags_only_where_create_run_would`, which also covers the two fixture items whose tags the demo box lacks (H-7). |
| D99 | T3's claim-time test uses `ANA-2` as the holder and `FEAT-3` (`["rust", "gpu"]`) as the refused item over `seed`'s primary repo, with no `touched_paths`. The plan's drop clause does not trigger (P-7). |
| D100 | Status-line assertions are byte-exact and include the worker's request name: `start_run: item <uuid>: missing tags: …` (P-6). |
| D101 | `EngineError::MissingTags`'s `#[error]` uses thiserror's field-expression argument (`.missing.join(", ")` in T1, `missing_tags_failure(.missing)` in T2). No hand-written `Display`. |
| D102 | T0's commit (a) carries the `pg_criteria.rs` borrow fix together with the `Copy` removal, and T0's gate is `cargo check --workspace --all-features --all-targets` (H-2). |
| D103 | The Postgres item statement is D80's text exactly, with no `closed_at` (a `queued` item's is already NULL), and the run statement uses `COALESCE(finished_at, $3)` like `finish_run`. |

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-41 | `prepare` infers the new `SELECT`'s `"tag!"` column differently from the shipped read and the build fails offline. | Low | It is the same expression as `pg/read.rs:1922-1926`; keep the `!` override. `prepare --check` is in T0's gate. |
| R-42 | T3's `drive_until` on the note is flaky if the retry lands late on a loaded box. | Low | `PATIENCE` is 30 s, and the retry follows the cancel task's end deterministically. Re-run alone after `df -h /` before believing it. |
| R-43 | A later milestone adds a `Claim` verdict that writes and forgets the worker's arms. | Low | `ClaimRefused`'s doc now says which verdicts it never carries, and H-3's review check is recorded here. |

`HANDOFF.md` (the MOD-7 entry's "criterion 14's `Unblock` half", the `.sqlx` count, the orch pin)
and the PRD are the main thread's to update at close (plan D88).
