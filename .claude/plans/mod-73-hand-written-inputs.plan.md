# Plan: MOD-73 hand-written document versions as step inputs

**Source**: HANDOFF `MOD-73` (from MOD-13, `docs/decisions/mod/mod-13.md` "Carried"; `R-ENT-12`, `R-ORCH-2`)
**Routed**: plan path via `/handoff-run` (1 criterion fired: C3, the two options the item names), accepted by the
maintainer 2026-10-03. Sandbox run `hr/MOD-73`.
**Complexity**: Small (one ranking rule, written three times — Postgres, the SQLite mirror, `MemStore` — plus the
tests that pin the old rule and one ANA-2 amendment)
**Status**: drafted, awaiting CONFIRM

## Summary

ANA-2 §4.2's gate-answer table says "edits the artifact" is `approved` plus a new `document` version, and MOD-13's
Docs `v` is how that version now gets written: by hand, `produced_by_step_id = NULL`. But §4.2's input resolver ranks
this run's output, then another run's, then a hand-written document, **whatever the version**, so the edit is shown
everywhere and never read by the next phase while any step-produced version of the kind exists.

MOD-73 changes the resolver's rule to: **per kind, the newer of (a) §4.2's step-produced pick and (b) the latest
hand-written version.** Since `document.version` is unique per `(item, kind)` and allocated in write order, "newer"
is "higher version", and a gate edit, written after the step's output, is what the next step reads. Everything §4.2
already promises about step-produced documents — this run first, losers excluded — is unchanged.

## Design decisions (proposed, maintainer may amend at CONFIRM)

- **D1: the rule is "newer hand-written version wins", not "`accept artifact` adopts it".** The item names both.
  Adopting would mean either re-stamping a hand-written row's `produced_by_step_id` (documents are append-only,
  `R-ENT-12`) or copying it into a new step-produced version, and it would only cover an edit made at a gate: an edit
  made after the run ended, before a re-run on the item, would still be ranked last. `accept artifact` is also the
  wrong hook — §4.8 guards it on a document *produced by this step* (a promoted chat's output), and the ordinary gate
  `approve` is what follows an edit. The read-side rule needs no write path, no engine change, and no new action, and it
  matches `documents_of_kinds` (prompt assembly's other read, latest by version) on every item without a fan-out
  loser or a second run's output in play.
- **D2: the two arms, exactly.** Per requested kind, over the item's documents of that kind:
  - **produced arm**: rows with `produced_by_step_id IS NOT NULL` whose step is not a loser
    (`s.id IS NULL OR s.selected IS NOT FALSE`), ranked as today — this run's `0`, another run's `1`, a row whose step
    is absent `2`, then `version DESC`; take the first.
  - **hand arm**: rows with `produced_by_step_id IS NULL`; take the highest version.
  - **answer**: the higher-versioned of the two picks (either may be absent; both absent is `None`).
  Consequences, all intended: a hand-written version older than this run's output loses to it (re-running over old
  history still reads this run's fresh output); one newer than this run's output wins (the gate edit); with no output
  of this run, the item's history is read newest-first across hand and other runs (a re-run reads an edit made at the
  previous run's gate); another run's output never overrides this run's (unchanged).
- **D3: "hand-written" is `d.produced_by_step_id IS NULL`, not `s.id IS NULL`.** On Postgres the two coincide
  (`fk_document_step ... ON DELETE SET NULL`), and `MemStore` drops steps only in `delete_project`, together with the
  project's items' documents, so it never holds a document whose step is absent either. They differ only on the
  mirror mid-pass (`cache/read.rs` `resolve_inputs` doc: `document` commits before `run_step`), where a fan-out
  **loser's** document briefly has no step. Keying the hand arm on the column keeps such a row in the produced arm at
  rank 2, so it cannot win on version over the winner; keying it on the join would let a mid-pass loser — often the
  highest version — beat a selected output. The mirror's mid-pass caveat stays as documented, not widened.
- **D4: one statement on both engines (blueprint H-14 holds).** The existing `ROW_NUMBER()` is partitioned by
  `(d.kind, d.produced_by_step_id IS NULL)` instead of `d.kind`, and an outer `ROW_NUMBER() OVER (PARTITION BY kind
  ORDER BY version DESC)` over the `rank_in_arm = 1` rows picks the higher version. No `DISTINCT ON`, no boolean
  `DESC NULLS LAST`; the Rust side of both SQL backends (request-order mapping, empty-`kinds` byte order) is untouched.
  Probed on Postgres (sandbox 5439) and SQLite 3.50 with an identical fixture: both return the same picks from two
  seats (see verified claims).
- **D5: no migration.** `0003_orchestration.sql`'s `COMMENT ON COLUMN step_graph_phase.input_kinds` ("preferring this
  run's own output") stays: it does not mention hand-written documents either way, and a comment-only migration is
  not worth a schema version. ANA-2 §4.2 is the authority and gets the amendment.
- **D6: the review loop inherits it.** §4.4 carries the rejecting review to the re-run implement through the same
  resolver (`engine.rs` §4.4 test at ~8398). A hand-written `review` written after the rejecting one is therefore what
  the implement reads — the same "edit is a new version" reading. `gate.rs::no_progress` reads reviews by step
  position, not through the resolver, and is unaffected (its hand-written `review` leg stays "not a review of this
  loop").

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Resolver, Mem | `crates/htui-core/src/store/mem.rs` `State::resolve_input` / `input_rank` (~4299-4331) | `min_by_key((rank, Reverse(version)))` over a filtered iterator; doc comment spells out the SQL it mirrors |
| Resolver, Pg | `crates/htui-store/src/pg/read.rs` `resolve_inputs` (~749-830) | `query_as!` with a nested `ROW_NUMBER()` subquery, `rank_in_kind = 1`; Rust applies request order via `BTreeMap` |
| Resolver, mirror | `crates/htui-store/src/cache/read.rs` `resolve_inputs` (~980-1060) | Same statement in `sqlx::query`, `json_each(?)` for the kind list, integer `selected` test |
| Conformance (write) | `crates/htui-core/src/store/conformance.rs` `write_document_allocates_its_version` (~11177) | `const CASE`, `store.write_document(new_document(..))`, `picked(run)` closure, `"{CASE}: ..."` messages |
| Engine parity test | `crates/htui-store/tests/cache.rs` `the_mirror_ranks_resolve_inputs_the_way_postgres_does` (~940) | `common::demo_db()` guard, raw `INSERT` ladder, mirror answer asserted equal to Postgres's and by id |
| Errors | trait `ReadStore::resolve_inputs` (`traits.rs` ~187-205) | Total read; backend failures only — unchanged |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/store/conformance.rs` | UPDATE | Reshape `write_document_allocates_its_version`'s preference leg to pin both arms (T1) |
| `crates/htui-core/src/store/mem.rs` | UPDATE | `resolve_input` two-arm rule; `input_rank` doc; flip the unit test's "outranks a later hand-written version" leg (T1) |
| `crates/htui-core/src/store/traits.rs` | UPDATE | `ReadStore::resolve_inputs` doc states the new rule (T1) |
| `crates/htui-store/src/pg/read.rs` | UPDATE | Statement + doc (T2) |
| `crates/htui-store/.sqlx/query-*.json` | CREATE + DELETE | New hash for the changed statement; the old entry goes (T2) |
| `crates/htui-store/src/cache/read.rs` | UPDATE | Statement + doc (T2) |
| `crates/htui-store/tests/cache.rs` | UPDATE | Parity test ladder: its v4 hand-written row now wins from `RUN_3`'s seat; assert the new picks (T2) |
| `crates/htui-orch/src/conformance.rs` | UPDATE | New engine case: a gate edit reaches the next step's prompt (T3) |
| `docs/ANA-2.md` | UPDATE | §4.2 resolver SQL + prose, gate-answer "edits the artifact" row note, amendment marker citing MOD-73 (T4) |

The old entry is `crates/htui-store/.sqlx/query-41886abe…312f7.json` (the only one containing `rank_in_kind`).

## Tasks

### T1: the rule, pinned (TDD) — `htui-core`
- **Action**: tests first. (a) In `write_document_allocates_its_version`, write the ladder as by-second v1, **by-hand
  v2**, by-third v3, then assert: `second_run` → by-hand (hand newer than this run's output), `third_run` → by-third
  (this run's output newer than hand), `RUN_1` → by-third (another run's newer output beats older hand); then write
  **by-hand v4** and assert every seat → v4; keep the `documents_of_kinds` contrast and the request-order / `None`
  legs. Update the case's doc line. (b) In `mem.rs::resolve_inputs_prefers_this_run_and_skips_a_loser`, the hand v3
  leg flips to "a later hand-written version outranks this run's output". Watch both fail, then rewrite
  `State::resolve_input` as D2 (two filtered picks, `max_by_key(version)`), update `input_rank`'s doc, and the trait
  doc.
- **Mirror**: existing `resolve_input` shape; conformance case style.
- **Validate**: `cargo test -p htui-core --all-features resolve_inputs write_document_allocates` (MemStore runs the
  conformance cases in-crate).

### T2: Postgres and the mirror — `htui-store` (after T1)
- **Action**: the parity test first — its existing ladder (fixture v1 hand, v2 winner, v3 loser, + v4 hand, v5
  `STEP_PLAN`) now answers v4 from `RUN_3`'s seat (hand newer than its v2) and v5 from `RUN_1`'s (v5 newer than hand
  v4); assert both, plus engine equality, and update its doc. Then rewrite both statements per D4 (pg keeps
  `s.selected IS NOT FALSE`, mirror keeps the integer form), update the two doc comments (the three-armed `CASE` text,
  the mid-pass paragraph per D3), and regenerate the offline entry against a migrated scratch DB
  (`docs/hr-sandbox.md` prepare variant; the compose `htui` DB is empty).
- **Mirror**: current statements; `.sqlx` regen recipe.
- **Validate**: `SQLX_OFFLINE=true cargo check -p htui-store --all-features`; `cargo test -p htui-store --all-features
  --test cache --test pg_conformance -- --test-threads=1` with `HTUI_TEST_DATABASE_URL` set.

### T3: the engine sees it — `htui-orch` (after T1; parallel with T2)
- **Action**: one new conformance case: a run parks at a gated phase with its output document; a hand-written version
  of that kind is written; the gate is answered `approved`; the next phase's stage 3 reads the hand-written version
  (assert on what the fake driver / step record captured of the assembled inputs, or on `resolve_inputs` at the run's
  seat if the harness exposes nothing better — the architect picks). Register it in the case list and dispatch match.
- **Mirror**: existing gate-answer cases in `htui-orch/src/conformance.rs`; `Box::pin` dispatch.
- **Validate**: `cargo test -p htui-orch --all-features -- --no-fail-fast` and grep the output for `SIGABRT`
  (the dispatch-stack memory note).

### T4: ANA-2 amendment — docs (parallel with T2/T3)
- **Action**: §4.2 "Input resolution, exactly": replace the SQL sketch with the two-arm statement, add one paragraph
  for the hand arm and D3's column-not-join choice, and a dated "Amended by MOD-73" marker per the doc's convention;
  in the gate-answer table, the "edits the artifact" row notes the edit is read by the next step because it is the
  newer version. No §4.8 change (`accept artifact` keeps its own guard).
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

**Independence** (file sets from the table): T1 {core conformance.rs, mem.rs, traits.rs}; T2 {pg/read.rs,
cache/read.rs, tests/cache.rs, .sqlx/}; T3 {htui-orch/src/conformance.rs}; T4 {docs/ANA-2.md}. T2/T3/T4 are pairwise
disjoint and each depends only on T1's rule, so they may run in parallel after T1.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
SQLX_OFFLINE=true cargo check --workspace --all-features
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # HTUI_TEST_DATABASE_URL / QDRANT set
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A test elsewhere relies on "hand-written ranks last" with a newer hand version | Low | Fact-checked: the demo fixture has no hand-written version newer than a step-produced one of its kind; the three pinning tests are named in Files to Change; full workspace gate with `--no-fail-fast` |
| Mirror and Postgres diverge on the nested window / boolean partition | Low | Probed on both engines (identical answers); the parity test asserts mirror == Postgres from two seats |
| Mid-pass mirror read admits a loser over a winner | Low | D3: the hand arm is keyed on the column, so a stepless row stays at rank 2 in the produced arm |
| Stale `.sqlx` entry left behind or prepare against the empty compose DB | Medium | Prepare against a migrated scratch DB; `SQLX_OFFLINE=true cargo check` gates it |
| Orch conformance dispatch near the 2 MiB stack | Low | One added arm, boxed like its neighbours; gate greps for `SIGABRT` |
| Someone wanted "this run's output always wins" for re-runs over old edits | Low | D2: an edit older than this run's output still loses; only a newer one wins |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- [ ] ANA-2 §4.2 and the three backends state the same rule

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `MemStore` ranks this run 0, another run 1, no step 2, then version DESC | ✓ | `mem.rs` `input_rank` + `resolve_input` `min_by_key((rank, Reverse(version)))` |
| Postgres uses `CASE WHEN s.id IS NULL THEN 2 WHEN s.run_id = $2 THEN 0 ELSE 1 END, d.version DESC` inside `ROW_NUMBER() PARTITION BY d.kind` | ✓ | `pg/read.rs` `resolve_inputs` |
| The mirror carries the same three-armed `CASE` with integer `selected` | ✓ | `cache/read.rs` `resolve_inputs` |
| `write_document_allocates_its_version` pins hand v3 losing to every run's output | ✓ | conformance.rs ~11255-11290: `(1, 2, 3)` ladder, three `picked` asserts |
| Two more tests pin the old rule | ✓ | `mem.rs` ~11262 "this run's output outranks a later hand-written version"; `tests/cache.rs` ~1015 "rank 2 never wins on version alone" (hand v4 > winner v2 from `RUN_3`) |
| `resolve_inputs_prefers_this_run_and_skips_losers` is unaffected | ✓ | its fixture hand-written row is v1, below the winner's v2 (`fixtures.rs` documents()) |
| No demo-fixture kind has a hand-written version newer than a step-produced one | ✓ | `fixtures.rs` `documents()`: research hand v1 < v2/v3; plan v1/v2 both by `STEP_PLAN`; verdict/summary/prd hand-only |
| `documents_of_kinds` takes the latest by version | ✓ | both conformance cases assert it as "plan D2's contrast" |
| Postgres cannot hold a document whose step is absent | ✓ | `0001_init.sql` ~595 `fk_document_step ... ON DELETE SET NULL` |
| `MemStore` removes steps only with the project (documents go too) | ✓ | `mem.rs` `delete_project` ~4117/4125 is the only `steps.retain`; no `steps.remove` |
| The mirror can briefly hold a stepless loser document | ✓ | `cache/read.rs` `resolve_inputs` doc ("Mid-pass ...") |
| The engine reads stage-3 inputs through `resolve_inputs` | ✓ | `engine.rs` ~5546 `.resolve_inputs(item, run.id, &phase.input_kinds)` → `InputDocument { kind, version, body }` |
| The TUI never calls `resolve_inputs` | ✓ | no hit under `crates/htui/` |
| `accept artifact` is guarded on a document produced by *this step* | ✓ | ANA-2 §4.8 ("whose `produced_by_step_id` is this step"); orch case `accept_artifact_needs_the_document_and_the_promotion` |
| `gate.rs::no_progress` reads reviews by step, not the resolver | ✓ | `gate.rs` `no_progress_review_counts_only_this_loops_reviews` — hand-written `review` leg is "not a review of this loop" |
| Nested `ROW_NUMBER()` with a boolean-expression partition runs on Postgres and SQLite and gives identical picks | ✓ | probe on sandbox Postgres 5439 and SQLite 3.50.4: seat 1 → research hand v2, plan hand v3 (loser v2 excluded), orphan v2; seat 2 → research run-2 v3 |
| A changed `query_as!` text needs a new `.sqlx` entry | ✓ | memory `sqlx-offline-hash-is-literal-query` (hash is the literal query) |
| The resolver's offline entry lives in `crates/htui-store/.sqlx/` | ✓ | `query-41886abe…json` is the only entry containing `rank_in_kind` |
| `0003`'s `input_kinds` column comment does not mention hand-written documents | ✓ | `0003_orchestration.sql:34-35` |
| T2/T3/T4 file sets are pairwise disjoint | ✓ | Files to Change table; intersection empty |
