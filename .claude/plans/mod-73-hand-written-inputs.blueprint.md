# Blueprint: MOD-73, hand-written document versions as step inputs

**Status**: **proposed** (2026-10-03). Plan deviations B-1 to B-8 (§0) and blueprint decisions
E1–E9 (§6) belong to this blueprint. A deviation marked **Blocker** means the plan, read literally,
produces a defect its own gate would show. The Fix column is what the implementer builds. The
plan's D1–D6 are binding and are not reopened.

**Plan**: `.claude/plans/mod-73-hand-written-inputs.plan.md` at `915738e5`, **confirmed** by the
maintainer 2026-10-03, fact-checked (step 3.5). Design authority: `docs/ANA-2.md` §4.2. Tasks are
cited as "MOD-73 T*n*" outside this file.

**Verified at**: HEAD `915738e5`, branch `hr/MOD-73`, clean tree, sandbox (`HR_SANDBOX=1`,
`HTUI_TEST_DATABASE_URL` set). Anchors were read through Gortex (`search`, `read`) and shell reads
where a hook allowed one. **Line numbers are pre-edit**: once a task commits to a file, a citation
into that file moves.

**Probes (this blueprint's exact SQL, not the plan's sketch).** §3.1's Postgres statement ran on the
sandbox Postgres (`psql -h localhost -p 5439 -U postgres`, `TEMP` tables, `PREPARE`/`EXECUTE`) and
§3.2's mirror statement ran on SQLite 3.50.4 (Python, in-memory, `json_each(?)`, integer
`selected`), over one fixture: the parity ladder of §3.4 (hand v1, `RUN_3` winner v2, loser v3,
hand v4, `STEP_PLAN`/`RUN_1` v5), T1's conformance ladder (by-second v1, hand v2, by-third v3, then
hand v4), and a stepless produced row (hand v1, orphan v2 whose step id joins nothing). Both engines
answered identically: parity `RUN_3` → v4, `RUN_1` → v5; ladder second → hand v2, third → v3, a
third run → v3; after hand v4 every seat → v4; orphan item → v2 (rank 2 in the produced arm, newer
than hand v1, which is plan D3's documented mid-pass shape). §5.2's per-kind ANA-2 sketch was also
run on Postgres: `RUN_3` → v4, `RUN_1` → v5, an unknown run → v5.

**Graphify**: `graphify-out/` isn't in this checkout, so nothing here comes from it.

**Coupling verdict.** The plan's order stands: **T1, then T2 ‖ T3 ‖ T4**. T1 owns every `htui-core`
file; T2 owns `htui-store` only; T3 owns `htui-orch` only (with B-1's second file, still inside the
crate); T4 owns `docs/ANA-2.md`. `htui-store` and `htui-orch` don't depend on each other (neither
lists the other in `[dependencies]` or `[dev-dependencies]`), so T2's `cargo sqlx prepare` and T3's
builds never compile the other's edits. File sets are pairwise disjoint (§1).

**Scope**:
- **No migration, no new dependency, no new type, no new trait method.** One `.sqlx` entry is
  replaced (the count stays **322**: one `D`, one `??`).
- **Changed behaviour**: `ReadStore::resolve_inputs` on all three backends (`MemStore`, `PgStore`,
  `CacheStore`), one rule (plan D2).
- **Pins that move**: `htui-orch` `CASES` 93 → **94** (two places, B-1). Store `CASES` (135) and
  `READ_CASES` (15) are unmoved: T1 reshapes an existing case.

**House style (carried)**:
- `SQLX_OFFLINE = "true"` (`.cargo/config.toml`): a `query_as!` whose text has no `.sqlx` entry does
  not compile. The entry hashes the **literal** query text, so the new entry lands in the **same
  commit** as the changed macro (H-1).
- Implementers commit after each step and stage only their own paths: never `-A`, never `stash`,
  never `--amend`. Every commit compiles. A red commit is allowed only as a tests-first commit
  inside one task.
- Integration tests need `--all-features` (or `--features testkit`/`test-support`). Without it,
  `tests/*.rs` run 0 tests and still report ok. Every Postgres-touching test run uses
  `--test-threads=1`.
- `clippy::all` warns and the gate is `-D warnings`; rustdoc denies broken intra-doc links.

---

## 0. Plan deviations (found against the tree)

| # | Blocker? | Plan says | Tree at `915738e5` | Fix |
|---|---|---|---|---|
| **B-1** | **Blocker** (two pins go red) | T3 file set: `{htui-orch/src/conformance.rs}`; "Register it in the case list and dispatch match." | `CASES.len()` is pinned at **93** twice: `conformance.rs:7293-7327` `cases_are_unique_and_counted` and `crates/htui-orch/tests/fake_conformance.rs:14-22` `cases_len_is_pinned`. One more name fails both. | T3 also owns `crates/htui-orch/tests/fake_conformance.rs`. Both pins go to **94** with the message extended (§4.5). T3's set is still disjoint from T2 and T4. |
| **B-2** | **Blocker** (the gate as written errors) | T1 Validate: `cargo test -p htui-core --all-features resolve_inputs write_document_allocates`. | `cargo test` takes one `TESTNAME` positional, so a second one is an argument error. `write_document_allocates_its_version` is also not a `#[test]`: store conformance cases run inside `tests/mem_store.rs::mem_store_conformance` (`run_all`, a fresh `MemStore::demo()` per case). | §1's T1 gate: `--test mem_store` for the cases, `--lib store::` for the `mem.rs` unit test and the cross-reference scanner, then the whole crate. |
| **B-3** | Non-blocker (stale doc) | — | The `CASES` doc says "Ninety-one" (`conformance.rs:322`), but the list holds 93 and its own recount line (`:326`) sums to 93. | T3 rewrites it to "Ninety-four" and appends `+ 1` to the recount (§4.5). |
| **B-4** | Non-blocker (wording) | Files to Change names three tests that pin the old rule. | A fourth message states it loosely: `conformance.rs:13830`, `resolve_inputs_prefers_this_run_and_skips_losers`, "another run's selected output still outranks the hand-written v1". The assertion still holds under D2 (produced pick v2 is newer than hand v1), but "outranks" now describes the wrong mechanism. | T1 rewords the message only (§2.4). The expected id is unchanged. |
| **B-5** | Non-blocker (ANA-2 completeness) | T4: §4.2 sketch, one paragraph, the marker, the gate-table row. | ANA-2 §4.2's retention paragraph (`ANA-2.md:418-422`) says an `ON DELETE SET NULL` would "reclassify a loser's document as hand-written and readmit it to this query". Under D2 such a row joins the **hand arm** and competes on version, so it could beat the selected output, not just trail it. No retention sweep is built yet (no `DELETE FROM run_step` in `crates/`), so this is latent. | T4 adds one sentence to that paragraph: the sweep's loser skip is now load-bearing (§5.4). |
| **B-6** | Non-blocker (noted, not changed) | T4 amends ANA-2 only. | Two concluded analyses quote the old `ORDER BY`: `docs/ANA-5.md:1283` (its point, that attempt 1's own output is a resolved input on attempt 2, still holds under D2) and `docs/ANA-10.md:1114` (a SQLite portability row, already superseded by blueprint H-14's explicit `CASE`). | Unchanged. ANA-2's marker is the authority. The close-out write-up names both as historical citations. |
| **B-7** | Non-blocker (close-out bookkeeping) | — | `HANDOFF.md:58` pins store `CASES` 134, `READ_CASES` 14, `htui-orch` `CASES` 92. The tree has 135, 15 and 93 (sibling merges since the recount). | Not a T1–T4 file. The close-out recount writes 135, 15, **94**, and 322 `.sqlx` (unchanged), and drops MOD-73 from the open list. |
| **B-8** | Non-blocker (environment) | T2: "regenerate the offline entry against a migrated scratch DB". | At HEAD the sandbox Postgres holds only `postgres`, `template0`, `template1`: no `htui_sqlx` (a sandbox restart wipes it; memory `hr-sandbox-restart-wipes-tmp-and-cache`). `cargo sqlx prepare` regenerates the whole `.sqlx` directory. | §3.5's recipe creates the database if absent and migrates it. The gate is exactly one `D` (the old `41886abe…` entry) and one `??` under `crates/htui-store/.sqlx`. |

### 0a. Hazards, each with its guard

| # | Hazard | Guard |
|---|---|---|
| **H-1** | `SQLX_OFFLINE=true`: the changed `query_as!` fails the build without its new entry. Regenerating without `--all-targets --all-features` deletes the test-only entries. | §3.5's recipe, verbatim from `docs/hr-sandbox.md:196-210`. The statement and the regenerated `.sqlx` land in one commit. Never touch the SQL text after `prepare` without re-running it. |
| **H-2** | `every_cross_referenced_test_name_exists` (`conformance.rs:16377-16483`) scans **every line** of `htui-core/src/store/conformance.rs` (strings included). A back-ticked `file.rs::name` must point at a file in its table (`conformance`, `mem`, `pg_criteria`, `box_identity`), and a back-ticked snake_case token with ≥4 `_` must be a fn in `conformance.rs` or `mem.rs`. | T1's new text in `conformance.rs` names no test by back-ticked path; in particular, never `cache.rs::the_mirror_ranks_…` (the scanner panics on an unknown file). `mem.rs` doc comments aren't scanned. |
| **H-3** | Between T1's green commit and T2's, the workspace is red on Postgres: `pg_conformance`'s run of the reshaped `write_document_allocates_its_version` expects the new picks from the old statement. | Expected. Don't run the full workspace gate until T2 is in. T3 and T4 gates don't touch Postgres. |
| **H-4** | Test stack (memory `htui-orch-test-stack-headroom`): `every_case_name_dispatches` runs every case on a 2 MiB test thread, and an unboxed arm adds its future to the dispatch frame. | The new case gets its own boxed frame, `input_case`, chained in `case` like `hand_back_case`, `persona_case` and `hardening_case` (§4.3). The gate runs `--no-fail-fast` and greps for `SIGABRT`/`overflow`. |
| **H-5** | T3's prompt assertion pins the section's exact bytes. | The bytes are `render::wrap` over `render::document` (`htui-core/src/prompt/render.rs:250-268`, `:302-313`): `<section name="documents:prd" kind="prd" version="N">`, LF, the body with trailing LFs trimmed (`content_of`, `:157-160`), LF, `</section>`. The case's bodies are one line with no `"`/`&`, so `attr` changes nothing. If the literal ever fails on formatting alone, split it into `kind="prd" version="N"` and the body, and don't loosen the negative assertion. |
| **H-6** | Parallel implementers on one tree (memory `parallel-fanout-hidden-file-coupling`). | §1's file sets are disjoint, and no task edits a snapshot, seed or `.sqlx` it doesn't own. T2 alone runs `cargo sqlx prepare`, from `crates/htui-store`. Before the close gate, check for orphaned test processes and home-dir debris. |
| **H-7** | Two picks could tie on version. | They can't: `document UNIQUE (item_id, kind, version)` holds on all three backends, and the arms partition a kind's rows. `max_by_key` and the outer `ROW_NUMBER` need no tiebreak. |

---

## 1. Build order and validation, at a glance

| Task | Files (owned exclusively) | Commits | Gate |
|---|---|---|---|
| **T1** rule, pinned | `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/traits.rs` | 2 (§2.6) | `cargo test -p htui-core --all-features --test mem_store -- --test-threads=1`; `cargo test -p htui-core --all-features --lib store:: -- --test-threads=1`; `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo clippy -p htui-core --all-targets --all-features -- -D warnings` |
| **T2** Postgres + mirror | `crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/cache/read.rs`, `crates/htui-store/tests/cache.rs`, `crates/htui-store/.sqlx/` (one D, one new) | 2 (§3.6) | `SQLX_OFFLINE=true cargo check -p htui-store --all-features --all-targets`; `(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features)`; `cargo test -p htui-store --all-features --test cache --test pg_conformance -- --test-threads=1 --nocapture 2>&1 \| grep -E "skipped\|panicked\|test result"` (no `skipped:` line); `git status --porcelain crates/htui-store/.sqlx` = one ` D`, one `??`; clippy `-p htui-store` |
| **T3** engine case | `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs` (B-1) | 1 (§4.6) | `cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 \| grep -E "SIGABRT\|overflow\|FAILED\|panicked\|test result"`; clippy `-p htui-orch` |
| **T4** ANA-2 amendment | `docs/ANA-2.md` | 1 (§5.6) | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| close | — | — | §7, on the real tree, after all four |

**Dependencies.** T2, T3 and T4 each depend only on T1's rule (T2's parity test and T3's case
assert it; T4 documents it). Intersections of the four sets are empty. T2 and T3 may run in
parallel; T4 needs no build at all.

---

## 2. T1: the rule, pinned (`htui-core`; plan D2, D3)

Tests first (§2.3, §2.4), watch them fail on the old rule, then §2.1 and §2.2.

### 2.1 `State::resolve_input` (`crates/htui-core/src/store/mem.rs:4312-4331`)

Replace the doc comment and body. `std::cmp::Reverse` stays path-qualified as today (it isn't
imported, so `unused_qualifications` has nothing to say). Two plain chains, not a shared closure: a
closure returning a borrowed `Filter` is a lifetime puzzle this doesn't need.

```rust
    /// One kind of ANA-2 §4.2's resolver as amended by MOD-73 (plan D2): the newer of two picks.
    ///
    /// The **step-produced** pick is the resolver as it was: rows with a `produced_by_step_id`
    /// whose step is not a fan-out loser, best [`State::input_rank`] first, then the highest
    /// version. The **hand-written** pick is the highest version with no producing step. The arm
    /// is keyed on the column, not on whether the step is still held (plan D3); this store never
    /// holds a document whose step is gone anyway, because only `delete_project` drops steps and
    /// it drops the project's items' documents with them. `(item, kind, version)` is unique, so
    /// the two picks never tie.
    fn resolve_input(&self, item: ItemId, run: RunId, kind: &str) -> Option<Document> {
        let produced = self
            .documents
            .iter()
            .filter(|document| document.item_id == item && document.kind == kind)
            .filter(|document| document.produced_by_step_id.is_some())
            .filter(|document| {
                // `s.selected IS NOT FALSE`: a fan-out loser is excluded, `NULL` is not.
                document
                    .produced_by_step_id
                    .and_then(|id| self.steps.get(&id))
                    .is_none_or(|step| step.selected != Some(false))
            })
            .min_by_key(|document| {
                (
                    self.input_rank(document, run),
                    std::cmp::Reverse(document.version),
                )
            });
        let by_hand = self
            .documents
            .iter()
            .filter(|document| document.item_id == item && document.kind == kind)
            .filter(|document| document.produced_by_step_id.is_none())
            .max_by_key(|document| document.version);
        produced
            .into_iter()
            .chain(by_hand)
            .max_by_key(|document| document.version)
            .cloned()
    }
```

### 2.2 `input_rank` doc (`mem.rs:4299-4301`) and the trait doc

`input_rank`'s body is unchanged. Its doc becomes:

```rust
    /// The step-produced arm's order (ANA-2 §4.2; MOD-73 plan D2), the SQL's three-armed `CASE`
    /// spelled out: this run's output 0, another run's 1, a document whose producing step is not
    /// held 2. [`State::resolve_input`] ranks only rows that have a `produced_by_step_id`; a
    /// hand-written document is the other arm and is never ranked here.
```

`ReadStore::resolve_inputs` (`crates/htui-core/src/store/traits.rs:186-191`). Replace the first
paragraph. The second paragraph ("Total, like the two list reads above…") and `# Errors` stay.

```rust
    /// ANA-2 §4.2's resolver as amended by MOD-73, which
    /// [`documents_of_kinds`](ReadStore::documents_of_kinds) is not (plan D2): per requested kind,
    /// the newer (higher `version`) of two picks over the item's documents of that kind. The
    /// **step-produced** pick takes rows whose producing step is not a fan-out loser
    /// (`selected IS NOT FALSE`), preferring one produced by a step of `run`, then another run's,
    /// each by version. The **hand-written** pick is the latest version with no producing step.
    /// So an edit written at a gate, after this run's output, is what the next step reads, and a
    /// hand-written version older than this run's output is not. One entry per kind in `kinds`
    /// order, a missing kind carried as `document: None`. An empty `kinds` means every kind the
    /// item has, in kind byte order.
```

**Other doc comments checked for the old rule** (the brief's list):

| Location | Says | Verdict |
|---|---|---|
| `conformance.rs:11177-11178` (case doc) | "ranks this run's output above another run's above a hand-written document" | **Changes**, §2.3 |
| `conformance.rs:13778-13782` (`resolve_inputs_prefers_this_run_and_skips_losers` doc) | winner wins, loser skipped, missing kind `None` | Unchanged. Its message at `:13830` changes (B-4) |
| `conformance.rs:16253-16257` (`hand_written_rows_round_trip` doc) | "asserts versions and ranking only" | Unchanged, still true |
| `conformance.rs:10502-10506` (`select_fanout_is_one_transaction` doc) | "the selection is what `resolve_inputs` reads" | Unchanged. No hand row on `FEAT-3`, and the leg at `:10614-10624` still answers the winner |
| `fixtures.rs:2558-2559` | "hand-written v1, the winner's v2 and the loser's v3: what `resolve_inputs` has to rank" | Unchanged, still true (v2 beats hand v1 from every seat) |
| `fixtures.rs:1238-1239` (`documents()` doc) | the `research` line is what §4.2's resolver is asserted against | Unchanged |
| `mem.rs:11209-11210` (unit test doc) | "prefers this run's output, skips a fan-out loser" | Extended, §2.5 |
| `model/document.rs:92`, `store/worker.rs:139` | links only | Unchanged |

### 2.3 Conformance case `write_document_allocates_its_version` (`conformance.rs:11177-11336`)

The version-allocation legs (`:11183-11234`) are unchanged. Doc lines `:11177-11178` become:

```rust
/// The version is allocated per `(item, kind)` inside the transaction (plan D6), and ANA-2 §4.2's
/// resolver, as amended by MOD-73, answers the newer of this run's preferred output and the latest
/// hand-written version.
```

The preference leg (`:11236-11335`) is rewritten. The two-run setup (`second_run`, `third_run`,
their steps, `:11239-11262`) stays as it is. From the three writes on:

```rust
    // The preference leg (MOD-73 plan D2): two runs of one item, each with a step, and a
    // hand-written version between their outputs. Nothing here is a fan-out loser, so
    // `selected IS NOT FALSE` admits every row and the two arms decide.
    // ... `second_run`, `third_run`, `second_step`, `third_step` exactly as today ...
    let by_second = store
        .write_document(new_document(ids::HTUI_ANA_2, "research", Some(second_step)))
        .await
        .expect(CASE);
    let by_hand = store
        .write_document(new_document(ids::HTUI_ANA_2, "research", None))
        .await
        .expect(CASE);
    let by_third = store
        .write_document(new_document(ids::HTUI_ANA_2, "research", Some(third_step)))
        .await
        .expect(CASE);
    assert_eq!(
        (by_second.version, by_hand.version, by_third.version),
        (1, 2, 3),
        "{CASE}: the hand-written version sits between the two runs' outputs, so both arms bite"
    );

    let picked = |run| async move { /* unchanged, `:11281-11289` */ };
    assert_eq!(
        picked(second_run).await,
        Some(by_hand.id),
        "{CASE}: a hand-written version newer than this run's output wins, and this run's v1 \
         was the step-produced pick over the other run's v3"
    );
    assert_eq!(
        picked(third_run).await,
        Some(by_third.id),
        "{CASE}: this run's output newer than the hand-written version wins"
    );
    assert_eq!(
        picked(ids::RUN_1).await,
        Some(by_third.id),
        "{CASE}: for a third run, the newer of another run's output and the hand-written version \
         wins"
    );
    assert_eq!(
        store
            .documents_of_kinds(ids::HTUI_ANA_2, &["research".to_owned()])
            .await
            .expect(CASE)
            .first()
            .map(|row| row.id),
        Some(by_third.id),
        "{CASE}: documents_of_kinds ranks by version alone — plan D2's contrast with \
         second_run's pick"
    );

    let later_hand = store
        .write_document(new_document(ids::HTUI_ANA_2, "research", None))
        .await
        .expect(CASE);
    assert_eq!(later_hand.version, 4, "{CASE}: the next call takes the next number");
    for run in [second_run, third_run, ids::RUN_1] {
        assert_eq!(
            picked(run).await,
            Some(later_hand.id),
            "{CASE}: a hand-written v4 is newer than every run's output, so {run} reads it"
        );
    }
    // ... `with_gap` request-order / `None` leg exactly as today (`:11312-11335`) ...
```

Why each pick bites (each is what one plausible wrong implementation would get wrong):
- `second_run` → `by_hand`: version alone answers `by_third`. An arm-less resolver (the old rule)
  answers `by_second`. A produced arm ranked by version only (losing `input_rank`) answers `by_third`.
- `third_run` → `by_third`: "hand always wins" would answer `by_hand`.
- `RUN_1` → `by_third`: "this run's output, else hand-written" would answer `by_hand`.
- The v4 loop: the old rule answers `by_second` / `by_third` / `by_third`.

`picked` is `Fn` (it moves a copy of `store: &S` into each future), so the loop may call it again.
`RunId` is `Display` (as in `tests/cache.rs`'s `"from {seat}"`).

### 2.4 B-4's message (`conformance.rs:13830`)

```rust
        "{CASE}: another run's selected v2 is newer than the hand-written v1, so it is read"
```

### 2.5 `mem.rs::resolve_inputs_prefers_this_run_and_skips_a_loser` (`mem.rs:11209-11300`)

The doc's first line gains the new rule:

```rust
    /// Plan D2: `resolve_inputs` prefers this run's output, skips a fan-out loser and reports a
    /// kind the item has no eligible row for — none of which `documents_of_kinds` does — and
    /// (MOD-73) reads a hand-written version newer than this run's output.
```

The hand leg (`:11262-11280`) flips. `FEAT-1`'s `plan` holds v1 and v2 by `STEP_PLAN` (`RUN_1`), so
the hand-written write is v3:

```rust
        let hand = store
            .write_document(a_document(ids::HTUI_FEAT_1, "plan", None))
            .await
            .expect("v3 lands");
        let preferred = store
            .resolve_inputs(ids::HTUI_FEAT_1, ids::RUN_1, &["plan".to_owned()])
            .await
            .expect("the resolver answers");
        assert_eq!(
            preferred[0].document.as_ref().map(|row| row.id),
            Some(hand.id),
            "a hand-written version newer than this run's output outranks it (MOD-73)"
        );
        assert_ne!(
            preferred[0].document.as_ref().map(|row| row.id),
            Some(ids::DOC_FEAT_1_PLAN_V2)
        );
```

The empty-`kinds` leg and the unknown-item leg after it are unchanged.

### 2.6 Commits (T1)

1. `test(mod-73): resolve_inputs pins the newer hand-written version` — §2.3, §2.4, §2.5. Red:
   `mem_store_conformance` and the unit test fail on the old `resolve_input`.
2. `feat(mod-73): resolve_inputs reads a newer hand-written version (MemStore)` — §2.1, §2.2.
   Green on the T1 gate.

---

## 3. T2: Postgres and the mirror (`htui-store`; plan D2, D3, D4)

Test first (§3.4). On Postgres it fails against the old statements.

### 3.1 `PgStore::resolve_inputs` (`crates/htui-store/src/pg/read.rs:749-830`)

The `wanted` prologue (`:769-782`) and the `BTreeMap` request-order epilogue (`:816-829`) are
unchanged. The `query_as!` text becomes this, with its bind list (`item`, `run`, `&wanted[..]`)
unchanged:

```sql
            SELECT id                  AS "id!: DocumentId",
                   item_id             AS "item_id!: ItemId",
                   kind                AS "kind!",
                   version             AS "version!",
                   title               AS "title!",
                   body                AS "body!",
                   produced_by_step_id AS "produced_by_step_id: StepId",
                   created_by          AS "created_by!: UserId",
                   created_at          AS "created_at!"
              FROM (SELECT arm.id,
                           arm.item_id,
                           arm.kind,
                           arm.version,
                           arm.title,
                           arm.body,
                           arm.produced_by_step_id,
                           arm.created_by,
                           arm.created_at,
                           ROW_NUMBER() OVER (
                               PARTITION BY arm.kind
                               ORDER BY arm.version DESC) AS rank_in_kind
                      FROM (SELECT d.id,
                                   d.item_id,
                                   d.kind,
                                   d.version,
                                   d.title,
                                   d.body,
                                   d.produced_by_step_id,
                                   d.created_by,
                                   d.created_at,
                                   ROW_NUMBER() OVER (
                                       PARTITION BY d.kind, d.produced_by_step_id IS NULL
                                       ORDER BY CASE WHEN s.id IS NULL      THEN 2
                                                     WHEN s.run_id = $2     THEN 0
                                                     ELSE 1 END,
                                                d.version DESC) AS rank_in_arm
                              FROM document d
                              LEFT JOIN run_step s ON s.id = d.produced_by_step_id
                             WHERE d.item_id = $1
                               AND d.kind = ANY($3::text[])
                               AND (s.id IS NULL OR s.selected IS NOT FALSE)) arm
                     WHERE arm.rank_in_arm = 1) ranked
             WHERE rank_in_kind = 1
```

Inside the hand arm every row has `s.id IS NULL`, so the `CASE` is a constant 2 and `version DESC`
picks the latest. Inside the produced arm the `CASE` is today's rank. The typed aliases and their
`!` overrides are kept verbatim, so `Document`'s field types are unchanged. Doc comment
(`:749-761`):

```rust
    /// ANA-2 §4.2's input resolver as amended by MOD-73 (plan D2): per kind, the newer of the
    /// step-produced pick (fan-out losers excluded, this run's own output preferred) and the
    /// latest hand-written version; one entry per requested kind in request order.
    ///
    /// The rank is written as an explicit `CASE` rather than the `(s.run_id = $2) DESC NULLS LAST`
    /// of §4.2's first sketch, and the mirror's statement carries the same three arms: `DESC NULLS
    /// LAST` over a boolean is not spelled or sorted alike on SQLite, and the two backends have to
    /// agree (blueprint H-14). Two `ROW_NUMBER()` windows rather than `DISTINCT ON`, for the same
    /// reason: the inner one, partitioned by `(kind, produced_by_step_id IS NULL)`, picks each
    /// arm's best; the outer one keeps the higher version per kind. The hand-written arm is keyed
    /// on the column, not on `s.id IS NULL` (MOD-73 plan D3). On Postgres the two coincide
    /// (`fk_document_step ... ON DELETE SET NULL`); the mirror is where they don't.
    ///
    /// The caller's order is applied in Rust, exactly as
    /// [`documents_of_kinds`](ReadStore::documents_of_kinds) applies it and for the same two
    /// reasons: it is an arbitrary permutation no `ORDER BY` expresses, and the empty-`kinds` case
    /// must be byte order rather than the database collation.
```

### 3.2 `CacheStore::resolve_inputs` (`crates/htui-store/src/cache/read.rs:979-1059`)

The prologue, the three `.bind` calls **in the same order** (`run`, `item`, the JSON kind list: the
`?` placeholders still appear in that order in the text) and the epilogue are unchanged. The
statement becomes:

```rust
        let rows = sqlx::query(
            "SELECT id, item_id, kind, version, title, body, produced_by_step_id, created_by, \
                    created_at \
               FROM (SELECT id, item_id, kind, version, title, body, produced_by_step_id, \
                            created_by, created_at, \
                            ROW_NUMBER() OVER ( \
                                PARTITION BY kind \
                                ORDER BY version DESC) AS rank_in_kind \
                       FROM (SELECT d.id, d.item_id, d.kind, d.version, d.title, d.body, \
                                    d.produced_by_step_id, d.created_by, d.created_at, \
                                    ROW_NUMBER() OVER ( \
                                        PARTITION BY d.kind, d.produced_by_step_id IS NULL \
                                        ORDER BY CASE WHEN s.id IS NULL      THEN 2 \
                                                      WHEN s.run_id = ?      THEN 0 \
                                                      ELSE 1 END, \
                                                 d.version DESC) AS rank_in_arm \
                               FROM document d \
                               LEFT JOIN run_step s ON s.id = d.produced_by_step_id \
                              WHERE d.item_id = ? \
                                AND d.kind IN (SELECT k.value FROM json_each(?) k) \
                                AND (s.id IS NULL OR COALESCE(s.selected, 1) <> 0)) \
                      WHERE rank_in_arm = 1) \
              WHERE rank_in_kind = 1",
        )
```

Doc comment (`:979-997`), first two paragraphs and the mid-pass paragraph (the integer-`selected`
paragraph between them is unchanged):

```rust
    /// The Postgres statement's twin, down to the three-armed `CASE` and the two arms (MOD-73
    /// plan D2, D4).
    ///
    /// Blueprint H-14: `(s.run_id = ?) DESC NULLS LAST` is not a spelling both engines sort the
    /// same way, so the rank is written out (this run's own output 0, another run's 1, a row whose
    /// step is absent 2), and each pick is a `ROW_NUMBER()` window, which is one statement Postgres
    /// and SQLite both run, rather than `DISTINCT ON`. The inner window is partitioned by
    /// `(kind, produced_by_step_id IS NULL)`, so a kind's step-produced rows and its hand-written
    /// rows each yield their best; the outer one keeps, per kind, the higher version of the two.
    /// The conformance read case and `tests/cache.rs`'s parity test are what prove the two agree.
    ///
    /// ... (`selected` paragraph unchanged) ...
    ///
    /// **Mid-pass the two engines can still disagree, and that is the refresher's shape, not this
    /// statement's.** [`run_pass`](crate::cache::refresh::run_pass) walks one table at a time and
    /// commits each batch on its own, with `document` seventh and `run_step` ninth, so a reader
    /// between those two commits sees a document whose producing step has not arrived: the
    /// `LEFT JOIN` yields `s.id IS NULL`, which passes the eligibility test and ranks the row 2
    /// **inside the step-produced arm**. The hand-written arm is keyed on the
    /// `d.produced_by_step_id IS NULL` column, not on the join (MOD-73 plan D3), so such a row
    /// never competes as hand-written. A fan-out **loser's** document can still be answered here
    /// where Postgres excludes it, when nothing in its arm ranks above it and it is newer than the
    /// hand-written pick. ANA-9 §6.2 promises whole-table consistency, not cross-table, so this is
    /// by design; a caller that cannot tolerate a loser's output must read Postgres.
```

### 3.3 Other tests that might assert the old ranking: **none**

Searched every `resolve_inputs` call site, every `produced_by_step_id: None` literal and every
`write_document(` in `htui-orch`, `htui-agent`, `htui-store/tests` and `htui/tests`:

| Site | What it does | Affected? |
|---|---|---|
| `htui-core` conformance `resolve_inputs_prefers_this_run_and_skips_losers` (`:13783-13870`), on Mem, Pg (`pg_conformance` `READ_CASES`) and the mirror (`cache.rs::the_mirror_passes_the_read_cases`) | Fixture only: hand v1 < winner v2 on `ANA-1`; `FEAT-1` `plan` has no hand row | No (wording only, B-4) |
| `htui-core` conformance `select_fanout_is_one_transaction` (`:10614-10624`) | `FEAT-3` `implementation`, no hand row | No |
| `htui-core` conformance `hand_written_rows_round_trip` | Writes hand `plan` v3 on `FEAT-1`; never resolves | No |
| `htui-orch` conformance `review_rejection_loops_then_escalates` (`:1758-1773`), `engine.rs::a_review_rejection_loops_then_escalates` (`:8394-8409`) | Assert only `review` `is_some()`; no hand row | No |
| `htui-orch` `gate.rs` `produce` (`:1528-1544`) | Hand `review` for `no_progress`, which reads by step (plan D6) | No |
| `htui-orch` `engine.rs::verdict` (`:7589-7601`), `closeout.rs` `summary` | A pure-function input; the close-out `summary` is no phase's `input_kinds` (`seed.rs` phases read `research`, `prd`, `plan`, `review`, `implement`, `reproduce`, `fix`) | No |
| `htui-agent` `conformance.rs:701`, `tests/recorder.rs:389` | Forwarders | No |
| `htui-store/tests/pg_criteria.rs:4433` | Version contention; never resolves | No |
| `htui-store/tests/cache.rs:1515` | Close-out resolution mirror; never resolves | No |
| `htui/tests/backlog.rs:2699`, `hand_written_pg.rs:160-197`, `runs_pg.rs:853` | TUI writes / close-out; no engine walk reads them (the TUI never calls `resolve_inputs`) | No |

The three pins the plan names (core conformance, `mem.rs` unit test, `tests/cache.rs`) are the only
ones that assert a hand-written version newer than a step-produced one.

### 3.4 `tests/cache.rs::the_mirror_ranks_resolve_inputs_the_way_postgres_does` (`:941-1035`)

The fixture (hand v1, `RUN_3` winner v2, loser v3) and the two inserted rows (hand v4, `STEP_PLAN`
v5) are unchanged, and so are the `winner` closure and its mirror-equals-Postgres assertion. The
three asserts at `:1018-1032` become two:

```rust
    assert_eq!(
        winner(ids::RUN_3).await,
        Some(hand_written_v4),
        "from RUN_3's seat the step-produced pick is its own v2 (rank 0 beats v5's rank 1), and \
         the hand-written v4 is newer, so v4 is read (MOD-73)"
    );
    assert_eq!(
        winner(ids::RUN_1).await,
        Some(other_runs_v5),
        "from RUN_1's seat v5 is the rank-0 row and newer than the hand-written v4, so the arms \
         are told apart and not merely ordered"
    );
```

The doc (`:941-962`) is rewritten to match. Keep the A-6 paragraph as it is, then:

```rust
/// `resolve_inputs`' three-armed `CASE` rank and its two arms (MOD-73) are the mirror's, not just
/// Postgres's (T2 audit).
///
/// ... (A-6 paragraph unchanged) ...
///
/// So the fixture's `research` ladder is extended here, in Postgres, with two rows that make each
/// arm decide something:
///
/// - **v4, hand-written** (`produced_by_step_id IS NULL`, the hand-written arm). It is newer than
///   `RUN_3`'s own v2 and older than v5, so it wins from one seat and loses from the other.
/// - **v5, produced by `STEP_PLAN`** (rank 1 from `RUN_3`'s seat, rank 0 from `RUN_1`'s). `RUN_1` is
///   the run `STEP_PLAN` belongs to and its `selected` is `NULL`, so the row is eligible from both
///   seats and only the rank tells them apart.
///
/// From `RUN_3` the answer is v4: the step-produced arm picks v2 (rank 0 beats a higher-versioned
/// rank 1), and v4 is newer. From `RUN_1` it is v5, rank 0 there and newer than v4. A mirror that
/// dropped the `CASE` would answer v5 from `RUN_3`; one that dropped the arm partition would answer
/// v2. Both are asserted equal to Postgres's own answer as well as by id, so a mirror that ranks
/// differently *or* a Postgres that does fails.
```

### 3.5 `.sqlx` regeneration (sandbox recipe, `docs/hr-sandbox.md:196-210`)

```bash
cd /home/mluigi/projects/htui/crates/htui-store
psql -h localhost -p 5439 -U postgres -Atc "SELECT 1 FROM pg_database WHERE datname = 'htui_sqlx'" | grep -q 1 \
  || psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features
cargo sqlx prepare --check -- --all-targets --all-features   # "potentially unused queries" warning is expected
git -C /home/mluigi/projects/htui status --porcelain crates/htui-store/.sqlx
#  D crates/htui-store/.sqlx/query-41886abee633784bb2113f815b9f07c55ddcb940fb980af357ec4d69196312f7.json
# ?? crates/htui-store/.sqlx/query-<new hash>.json        <- exactly these two lines
```

Don't use the compose `htui` database, which is empty (memory `sqlx-prepare-needs-migrated-scratch-db`).
Stage with explicit paths: `git add crates/htui-store/.sqlx/query-41886abe…json` (records the
deletion) and `git add crates/htui-store/.sqlx/query-<new>.json`. If `status` shows any other
`.sqlx` line, `prepare` ran without `--all-targets --all-features` or against the wrong database:
re-run, and don't commit until only those two lines remain.

### 3.6 Commits (T2)

1. `test(mod-73): the mirror parity ladder reads the newer hand-written version` — §3.4. Red on
   Postgres.
2. `feat(mod-73): resolve_inputs reads a newer hand-written version (Postgres, mirror)` — §3.1,
   §3.2, §3.5 together (H-1). Green on the T2 gate, including `pg_conformance` (H-3 clears).

---

## 4. T3: the engine sees it (`htui-orch`; plan D1, D2)

### 4.1 Harness facts this case relies on

- `CaseHarness::fresh` gives a `FakeOrchestrator::demo()`; `Orchestrate::store()` is a `MemStore`.
  The suite has **two bindings, both Mem**: the in-crate `tests::Demo`
  (`every_case_name_dispatches`, `run_all_walks_the_list`, `conformance.rs:7274-7344`) and
  `tests/fake_conformance.rs`'s `Demo`. There is **no Postgres binding** of the orch suite
  (`Orchestrate::store` returns `&MemStore` by design, `conformance.rs:66-70`). T2's
  `pg_conformance` is what carries the rule onto Postgres.
- `FEAT-3`'s `feature` graph: `prd` (no inputs) → `plan` (`input_kinds = ["prd"]`) → `implement` →
  `review`, every phase gated `always` (`feat_walks_end_to_end`). `free_feat_3` cancels the seeded
  `RUN_2` so the item can start. The fixture holds no `FEAT-3` document.
- `ScriptedStep::done_with_output(body)` makes the fake write the step's output document with
  `produced_by_step_id = Some(step.id)` (`fake.rs:1804-1829`).
- Approve's guard reads `Engine::output_of` (`engine.rs:6280-6298`): the newest document of
  `output_kind` **produced by this step**, over `documents()` (every version). A newer hand-written
  version doesn't hide the step's own, so approve is allowed.
- Stage 3 (`engine.rs:5542-5559`) maps `resolve_inputs` to `InputDocument { kind, version, body }`,
  and `render::document` + `render::wrap` put it in the prompt as
  `<section name="documents:<kind>" kind="<kind>" version="N">`. Every seeded template carries
  `{{documents}}` (`prompt/defaults.rs`).
- `prompt_text(orch, step)` (`conformance.rs:2797-2812`) returns the scrubbed text of the step's
  seq-0 `prompt` event: the bytes the agent was sent.

**Observable chosen: the next step's recorded prompt text** (strongest: it is what the agent read,
and it carries both the version and the body). The alternatives are weaker. A `resolve_inputs`
call at the run's seat is only the store, not the engine. `prompt_sections` names `documents:prd`
but carries no version. The step row has no input record. The trim record is a budget report. The
case asserts the prompt and also cross-checks the resolver at the seat (E7).

### 4.2 The case (after `approve_needs_the_output_document`, before `:1691`)

```rust
/// MOD-73 (ANA-2 §4.2 as amended; §4.2's gate-answer row "edits the artifact"): a version of the
/// parked step's `output_kind` written by hand at the gate (`produced_by_step_id` `NULL`, as the
/// Backlog's Docs `v` writes it since MOD-13) and then `approved` is what the next phase's stage 3
/// reads, because it is newer than the step's own output.
///
/// The observable is the next step's recorded prompt, the bytes its agent was sent: the
/// `documents:prd` section carries the edit's version and body, and the step's own body appears
/// nowhere in it. The resolver is asserted at the run's seat too, so the prompt and §4.2 cannot
/// disagree unnoticed.
async fn a_gate_edit_is_what_the_next_phase_reads<H: CaseHarness>(harness: &H) {
    const WRITTEN: &str = "PRD as the prd step wrote it.";
    const EDITED: &str = "PRD as the human edited it at the gate.";
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.script("prd", 1, ScriptedStep::done_with_output(WRITTEN));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "`prd` parks at its `always` gate with its output written"
    );
    let steps = steps_of(&orch, run).await;
    let parked = at(&steps, 0, 1).clone();
    let written = orch
        .store()
        .documents(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|head| head.kind == "prd" && head.produced_by_step_id == Some(parked.id))
        .expect("the parked step wrote its `prd`");

    // The edit: a new version, by hand, while the run is parked (R-ENT-12: append-only).
    let edited = orch
        .store()
        .write_document(NewDocument {
            id: DocumentId::new(),
            item_id: ids::HTUI_FEAT_3,
            kind: "prd".to_owned(),
            title: "prd (edited at the gate)".to_owned(),
            body: EDITED.to_owned(),
            produced_by_step_id: None,
            created_by: ids::USER,
            created_at: orch.clock().now(),
        })
        .await
        .expect("the item and the user exist");
    assert!(
        edited.version > written.version,
        "the edit is the newer version: v{} after the step's v{}",
        edited.version,
        written.version
    );

    let (answered, rest) = answer(&orch, run, GateAnswer::Approved).await;
    assert_eq!(answered.id, parked.id, "the approve answered the edited step");
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "approve walked on: `plan` ran and parked"
    );

    let steps = steps_of(&orch, run).await;
    let prompt = prompt_text(&orch, at(&steps, 1, 1).id).await;
    let section = format!(
        "<section name=\"documents:prd\" kind=\"prd\" version=\"{}\">\n{EDITED}\n</section>",
        edited.version
    );
    assert!(
        prompt.contains(&section),
        "`plan`'s stage 3 read the edit, v{}:\n{prompt}",
        edited.version
    );
    assert!(
        !prompt.contains(WRITTEN),
        "and not the step's own output, which the edit superseded:\n{prompt}"
    );

    let inputs = orch
        .store()
        .resolve_inputs(ids::HTUI_FEAT_3, run, &["prd".to_owned()])
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        inputs
            .first()
            .and_then(|input| input.document.as_ref())
            .map(|document| document.id),
        Some(edited.id),
        "§4.2's resolver at the run's seat answers the edit"
    );
}
```

Under the pre-T1 rule the `plan` prompt carries `WRITTEN` at v1 and the first `assert!` fails, so
the case bites by construction. Imports: add `DocumentId` and `NewDocument` to the first
`htui_core::model::{…}` list (`conformance.rs:21-26`). `WriteStore as _`, `ReadStore as _`,
`Clock as _` and `ids` are already imported.

### 4.3 Registration and dispatch

- **`CASES`** (`:397-603`): append after `"a_promoted_shared_serialized_step_keeps_other_runs_off_its_repo",`:
  ```rust
      // MOD-73 plan D1, D2 (ANA-2 §4.2 as amended): a version written by hand at a gate, then
      // approved, is the next phase's stage-3 input.
      "a_gate_edit_is_what_the_next_phase_reads",
  ```
- **A new frame** after `hardening_case` (`:921-935`), the house pattern (H-4):
  ```rust
  /// MOD-73's case, in a frame of its own for [`hand_back_case`]'s reason.
  fn input_case<'a, H: CaseHarness>(
      name: &str,
      harness: &'a H,
  ) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
      Some(match name {
          "a_gate_edit_is_what_the_next_phase_reads" => {
              Box::pin(a_gate_edit_is_what_the_next_phase_reads(harness))
          }
          _ => return None,
      })
  }
  ```
- **`case`** (`:624-629`): add `.or_else(|| input_case(name, harness))` after the `hardening_case`
  line, before `.unwrap_or_else(|| earlier_case(name, harness))`.
- `earlier_case`'s `# Panics` list (`:632-635`) already omits `hardening_case`. Rewrite it as "…
  nor any of the frames [`case`] tries first", so it doesn't go stale again.

### 4.4 Which harnesses run it

Both Mem bindings: the in-crate `every_case_name_dispatches` / `run_all_walks_the_list` and
`tests/fake_conformance.rs::fake_orchestrator_passes_every_case`. Not Postgres (§4.1). Not
`tests/gix_isolator.rs`, which doesn't iterate `CASES`.

### 4.5 Pins (B-1, B-3)

- `conformance.rs:322`: "Ninety-one," → "Ninety-four,".
- `:326`: append ` + 1` (the recount sums to 94).
- After the `**One for MOD-37 milestone 4**` paragraph (`:395-396`), add:
  ```rust
  /// **One for MOD-73** (plan D1, D2): a version of the parked step's output kind written by hand
  /// at the gate, then approved, is what the next phase's prompt carries.
  ```
- `cases_are_unique_and_counted` (`:7293-7327`): `93` → `94`. Prefix `… + 5 + 1` → `… + 5 + 1 + 1`.
  Append `, and MOD-73's one (a gate edit read by the next phase, plan D2)` before the closing `"`.
- `tests/fake_conformance.rs:17-21`: `93` → `94`. The message ends `…, and MOD-37 M4's R-49
  admission pin 93, and MOD-73's gate edit read by the next phase 94`.

### 4.6 Commit (T3)

One commit: `test(mod-73): a gate edit is what the next phase reads (orch conformance)`. §4.2–§4.5
together. It is green on landing because T1 already holds the rule.

---

## 5. T4: ANA-2 §4.2 amendment (`docs/ANA-2.md`)

**Marker convention** (ANA-2's own): an italic lead-in paragraph placed directly after the amended
text, `*Amended by MOD-4 milestone 6, 2026-09-25 (plan D161, R-42):* …` (`:560`, `:603`, `:1220`,
`:1235`, `:1275`), and an inline parenthetical in table cells, `(amended by MOD-4 milestone 6,
2026-09-25)` (`:558`, `:595`, `:1610`). The section-top block-quote form (`> **Amended by MOD-38
(2026-09-25, …).**`, `:522`) is for a change that rewrites a whole section, which this one doesn't.
MOD-73 has no write-up yet. The marker cites the plan path, which exists, so the validator's
cross-link check (`validate-workflow-docs.sh`, back-ticked `*.md` must resolve) passes.

### 5.1 Options table, adopted row (`:371`)

Append to the Reason cell: ` Amended by MOD-73, 2026-10-03: a hand-written version newer than that
pick wins (below).`

### 5.2 The SQL sketch (`:397-407`), replaced

```sql
SELECT arm.*
  FROM (SELECT d.*,
               ROW_NUMBER() OVER (
                   PARTITION BY d.produced_by_step_id IS NULL          -- two arms: step-produced, hand-written
                   ORDER BY CASE WHEN s.id IS NULL     THEN 2
                                 WHEN s.run_id = $run  THEN 0          -- this run's own output first
                                 ELSE 1 END,
                            d.version DESC) AS rank_in_arm
          FROM document d
          LEFT JOIN run_step s ON s.id = d.produced_by_step_id
         WHERE d.item_id = $item AND d.kind = $kind
           AND (s.id IS NULL OR s.selected IS NOT FALSE)) arm   -- hand-written, winner, or fan_out = 1
 WHERE arm.rank_in_arm = 1
 ORDER BY arm.version DESC                                       -- the newer of the two picks
 LIMIT 1;
```

(Per kind, as the sketch always was; the backends do all kinds in one statement with an outer
`ROW_NUMBER() OVER (PARTITION BY kind …)`. Probed, see header.)

### 5.3 The paragraph after it (`:409-412`)

The first sentence (`s.selected IS NOT FALSE` …) stays. The second becomes:

> Inside the step-produced arm, `WHEN s.run_id = $run THEN 0` is the rule that a phase reads what
> *this* run produced when it exists and falls back to the item's history otherwise, which is what
> makes a re-run of a graph on an item that already has documents behave sensibly.

Then a new paragraph:

> *Amended by MOD-73, 2026-10-03 (plan D1-D4, `.claude/plans/mod-73-hand-written-inputs.plan.md`):*
> the first sketch ordered `(s.run_id = $run) DESC NULLS LAST, d.version DESC`, which ranks a
> hand-written document after any run's output whatever its version. The gate table's "edits the
> artifact" (a new version followed by `approved`, below) was therefore shown everywhere and never
> read by the next phase while a step-produced version of the kind existed. The resolver now takes
> two picks per kind and answers the higher version: the **step-produced** pick, ranked as before
> (this run's output, then another run's, then version), and the **hand-written** pick, the latest
> version with `produced_by_step_id IS NULL`. `document.version` is allocated in write order per
> `(item, kind)`, so newer is higher. An edit written after this run's output wins. A hand-written
> version older than this run's output loses to it. With no output of this run, the item's history
> is read newest first across hand-written versions and other runs' output. Another run's output
> still never overrides this run's. The arm is keyed on the column, not on `s.id IS NULL`. On
> Postgres the two coincide (`ON DELETE SET NULL`), but the offline mirror can briefly hold a fan-out
> loser's document without its step mid-refresh, and keyed on the join that row could win on version
> over the selected output. Both SQL backends run this as one statement, two `ROW_NUMBER()` windows
> rather than `DISTINCT ON` or a boolean `DESC NULLS LAST`, and `MemStore` mirrors it. `accept
> artifact` (§4.8) is unchanged: it still needs a document produced by the promoted step.

### 5.4 The retention paragraph (`:418-422`, B-5)

After "…keeping the loser row … while still dropping its `session_event` rows.", insert:

> Since MOD-73 the skip is load-bearing in a second way: a document whose `produced_by_step_id` was
> nulled joins the hand-written arm and competes on version, so a readmitted loser could be read
> over the selected output, not merely after it.

### 5.5 Gate-answer table (`:466`)

```markdown
| edits the artifact | `approved` + a new `document` version, which the next phase reads because it is newer than the step's output (§4.2's resolver, amended by MOD-73, 2026-10-03) | `done` | `running` |
```

§4.4's carried-review row (`:750`, "§4.2's resolver picks the latest non-loser version, which is the
rejecting review") is **unchanged**. Its wording already matches the amended rule (plan D6: a
hand-written `review` written after the rejecting one is the latest non-loser version). §4.8 is
unchanged (plan T4).

### 5.6 Commit (T4)

`docs(mod-73): ANA-2 §4.2 - a newer hand-written version is the input`. Gate:
`bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` (no new finding). No test reads
`docs/ANA-2.md`. Line citations of ANA-2 elsewhere (`docs/ANA-2.md:2085` …, already drifted) move
by the inserted lines; nothing pins them.

---

## 6. Blueprint decisions

| # | Decision | Why |
|---|---|---|
| E1 | Mem: two plain iterator chains and `into_iter().chain().max_by_key(version)` | It reads like D2 line by line, and it avoids a closure returning a borrowed `Filter` (§2.1) |
| E2 | Pg/mirror: the outer window over an inner window, explicit column lists, no `arm.*` | The `query_as!` aliases stay byte-identical in shape, and the mirror's `document_of` reads columns by name. Probed on both engines (header) |
| E3 | The outer `ROW_NUMBER` is ordered by `version DESC` only | Versions are unique per `(item, kind)`, so it never ties (H-7) |
| E4 | Keep the `rank_in_kind` name for the outer rank and add `rank_in_arm` | The epilogue and every doc that says "`rank_in_kind = 1`" keep their meaning |
| E5 | T1's ladder has the hand row **between** the two runs' outputs | One write order makes all three seats answer differently from both the old rule and "version alone" (§2.3) |
| E6 | T3's observable is the recorded prompt's `documents:prd` section, version and body, plus a negative on the step body | It is the engine's actual output to the agent. A store read could pass with the engine bypassing it |
| E7 | T3 also asserts `resolve_inputs` at the run's seat | It ties the prompt to §4.2, so a future prompt path that reads documents some other way is caught |
| E8 | T3's case gets its own boxed `input_case` frame | The house pattern for stack headroom (H-4). `hardening_case`'s doc names MOD-37 alone |
| E9 | T4 uses ANA-2's inline italic marker and cites the plan path | It mirrors the doc's own mid-section amendments, and the validator's link check resolves the path |

---

## 7. Close-out gate (plan § Validation, on the real tree)

```bash
df -h .                                                     # target/ growth (memory: disk pressure)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
SQLX_OFFLINE=true cargo check --workspace --all-features --all-targets
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx \
   cargo sqlx prepare --check -- --all-targets --all-features)
git ls-files crates/htui-store/.sqlx | wc -l                # 322
cargo test -p htui-core --all-features -- --test-threads=1
cargo test -p htui-store --all-features --test pg_conformance --test cache -- --test-threads=1 --nocapture 2>&1 \
  | grep -E "skipped|panicked|test result"                  # no "skipped:" line: Postgres really ran
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 | grep -E "SIGABRT|overflow|test result"
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | grep -E "SIGABRT|FAILED|test result"
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

If `qdrant_live` times out under the full workspace load, re-run it serially before calling it a
regression (memory `qdrant-live-times-out-under-load`). Close-out bookkeeping (not T1–T4): B-6's two
historical citations go into the write-up, and B-7's HANDOFF recount.
