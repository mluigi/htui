# Plan: MOD-150 M2 — Close-out records duplicates and supersedes

**Source PRD**: `.claude/prds/mod-150-link-kinds.prd.md`
**Selected Milestone**: 2 — Close-out records duplicates and supersedes
**Complexity**: Medium
**Routing**: PRD path (resumed at `/plan` for M2), ultracode not needed (`/handoff-run`, 2026-11-13).

## Summary

`R-ENT-9`: "a person's close-out as `duplicate` or `superseded` records the link to the item they
name". Today `close_out` closes an item as `duplicate` or `superseded` with no link at all. After M2,
both stores refuse that close-out unless the link exists or the person names an item:

- **`duplicate`** needs a live `duplicates` link from the item.
- **`superseded`** needs a live `supersedes` link to the item.
- **Otherwise** the person names the item, and the link is written in the close-out's own transaction.

In the Runs pane's close-out flow, a new "name the item" stage appears between the warning and the typed
key, and only when no live link exists. The warning line shows the link that will stand.

## Decisions

- **D1 — Store API.** `WriteStore::close_out` and `WorkerStore::close_out` gain `names: Option<ItemId>`
  right after `resolution`:
  `close_out(item, resolution, names, summary, commits)`. Every impl and wrapper (MemStore ×2, PgStore
  ×2, Writer ×2, `UsageSpy`, `SpyStore`) forwards it. Every existing caller passes `None`.
  - **Why an explicit argument, not a new method:** `WorkerStore` returns `impl Future + Send`, so a
    provided default method would need a Send proof per impl. The churn is mechanical: about 30 test
    call sites, the `close_out(` text-search counts.
- **D2 — The link rule** runs after the close-out law (`closes_from`) and before the commit-step probes,
  in this order:
  1. `names` is `Some` but `resolution` is neither `duplicate` nor `superseded` →
     `Constraint(close_out_names_no_link(resolution))`.
  2. `names == item` → `Constraint(self_link(item))`.
  3. The named item is missing → `NotFound { entity: "item", id }`.
  4. The named item is outside reach → `Constraint(link_outside_reach(named))`. Reach is
     `propose_link`'s, measured from the closing item's project: Mem `link_reaches(item.project, named.project)`,
     Pg the same reach SQL with `$2 = item.project_id`.
  5. **`duplicate`:**
     - With a name: a live `duplicates` from the item to another item →
       `Constraint(second_canonical(item, current))`. Otherwise upsert `item -duplicates-> named`.
     - Without a name: a live `duplicates` from the item, or `Constraint(close_out_needs_a_link(item, resolution))`.
  6. **`superseded`:**
     - With a name: upsert `named -supersedes-> item`. `supersedes` is multi-valued, so an existing
       live one from another item is no refusal.
     - Without a name: a live `supersedes` to the item, or `Constraint(close_out_needs_a_link(item, resolution))`.

  Every refusal writes nothing. The existing refusals keep their order and sentences, so
  `close_out_resolution_law` and `close_out_refuses_a_live_run` still pass unchanged.
- **D3 — The upsert.** It works like `propose_link`'s on the PK `(from, to, kind)`, with
  `proposed_by_step_id = NULL` (a person: `ItemLink` doc "None means a human or the importer").
  - **A live row** keeps its proposer: it is idempotent and nothing changes but `updated_at`.
  - **A tombstone** revives with a NULL proposer.
  - **On Pg**, the upsert may reuse `propose_link`'s INSERT text byte-for-byte with `$4` bound to
    `Option::<Uuid>::None`, if sqlx's macro accepts an `Option` there. That reuses the `.sqlx` entry
    (memory: offline hash = literal query). If it does not, write a new query with a literal `NULL` and
    run `cargo sqlx prepare`.
- **D4 — Pg concurrency.**
  - **`duplicate` with a name** takes `propose_link`'s per-item advisory lock
    (`format!("item_link:duplicates:{item}")`, the same `pg_advisory_xact_lock(hashtextextended($1, 0))`
    text) before the second-canonical check. This holds for the reason review R1 gave in M1:
    `uq_item_link_one_canonical` would otherwise trip as 23505 on a concurrent agent proposal.
  - **`superseded`** takes no lock: no partial unique backs it, and a concurrent identical insert meets
    the PK upsert.
  - **An outgoing `duplicates` cannot be withdrawn concurrently.** Withdrawal needs a step of a live run
    of the item, `close_out` refuses a live run, and `create_run` takes the item row lock that
    `close_out` holds.
  - **An incoming `supersedes` can be.** Another item's run may withdraw it between the check and the
    commit. Accepted: that is the agent's own later act, and MOD-155 makes `supersedes` pending anyway.
- **D5 — New sentences** go in `store/traits.rs`, beside `resolution_not_closable`, as
  `#[must_use] pub fn … -> String`:
  - `close_out_needs_a_link(item, resolution)` →
    "item {item} closes as `duplicate` only with a live `duplicates` link from it or the item it
    duplicates named (R-ENT-9)". The `superseded` wording: "… only with a live `supersedes` link to it
    or the item that supersedes it named (R-ENT-9)".
  - `close_out_names_no_link(resolution)` → "close-out as `{resolution}` records no link; only
    `duplicate` and `superseded` name an item (R-ENT-9)".
- **D6 — `WorkerStore::links(id, hops) -> Result<LinkGraph>`.** A new method that forwards to
  `ReadStore::links` in its three impls (`MemStore`, `PgStore`, `Writer`). It is the engine's only way
  to read links: `WorkerStore` is its store bound (`engine.rs:516`).
- **D7 — Orchestrator.**
  - **`Command::CloseOut`** gains `names: Option<String>`: the key as the person typed it, qualified
    `slug:KEY` accepted.
  - **`Engine::close_out`** resolves it with `WorkerStore::item_by_key(reads.item.project_id, key)`. An
    unknown key → `EngineError::Store(StoreError::NotFound { entity: "item", id: key })`. It then
    passes `Some(id)` to the store, which decides everything else (D2).
  - **`closeout::Preview`** gains `duplicate_of: Option<String>` (the live canonical) and
    `superseded_by: Vec<String>` (live supersessors, sorted). Both are read in
    `Engine::close_out_preview` from `links(item, 1)`, and `closeout::preview` gets them as an
    argument. A label is `KEY` when the linked item is in the closing item's project, else `slug:KEY`
    (`LinkNode.project_slug`).
- **D8 — TUI (Runs pane).**
  - **`CloseOutStage` gains `Naming { preview, field }`.** From `Warn`, `ConfirmYes` enters `Naming`
    when the picked resolution is `duplicate` with no `duplicate_of`, or `superseded` with an empty
    `superseded_by`. Otherwise it enters `Typed` as today.
  - **In `Naming`:** the lines are the summary plus "duplicate of which item? type its key:" or
    "superseded by which item? type its key:". A non-empty submit enters `Typed` with the key held.
    An empty submit puts an error on the status line and stays. `Esc` returns to browse.
  - **`Typed`** carries `names: Option<String>`, and its submit sends `Command::CloseOut { item,
    resolution, names }`.
  - **`close_out_summary`** appends " · duplicate of K" / " · superseded by K[, K…]". K is the live
    link, or else the typed name. It appends nothing for other resolutions, so no existing snapshot
    moves.
  - **`Naming` is a text field like `Typed`:** it shares `TYPED_HINT`, the field stack and
    `RUNS_CONFIRM`'s mapping (`runs.rs:765`). No new key: every key stays in `keys.toml` (memory: every
    key configurable).
  - **An unknown key** comes back as the engine's error, on the path a failed `CloseOut` already takes.
- **D9 — Unchanged.**
  - No migration: the M1 CHECK already lists both kinds.
  - No fixture rows.
  - The close-out `summary` document's body.
  - Readiness: it reads only `blocked_by`, verified in M1.
  - The cache: it mirrors `item_link` by its `updated_at`/`deleted_at` cursor, so rows written inside
    `close_out` flow like any other.
- **D10 — PRD open question 3 (close-out surfaces).**
  - **The Runs pane is the only one.** The worker relays `Command::CloseOut { item, .. }` untouched
    (`htui-worker/src/runtime.rs:2845`, `views.rs:78`).
  - **No other close-out exists:** no CLI, headless or waiting-row close-out.
  - **MOD-135's future `CloseOut` waiting rows** send the same command, `names` included.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming / refusal sentences | `crates/htui-core/src/store/traits.rs:3948` `resolution_not_closable` | `#[must_use] pub fn … -> String`, one doc line naming the rule |
| Errors | `crates/htui-store/src/pg/write.rs:9258` `PgStore::close_out` | every refusal decided before the first write, inside the one transaction |
| Link write (Pg) | `crates/htui-store/src/pg/write.rs:10103` `PgStore::propose_link` | reach SQL, advisory lock, second-canonical query, `ON CONFLICT` upsert |
| Link write (Mem) | `crates/htui-core/src/store/mem.rs:9083` `State::propose_link` | `link_reaches`, `live_other`, position-then-revive on `self.links` |
| Close-out (Mem) | `crates/htui-core/src/store/mem.rs:8366` `State::close_out` | refusals first, then writes under the same `now` |
| Tests (store) | `crates/htui-core/src/store/conformance.rs:15388` `close_out_resolution_law`; `:33280` `duplicates_is_single_valued` | `CASE` const, `item_row`, `live_link`, registered in `CASES` (`:111`) |
| Tests (orch) | `crates/htui-orch/src/conformance.rs:7362` `close_out_writes_one_summary_and_closes_the_item` | `CaseHarness` case, registered in `CASES` (`:673`) |
| Tests (preview) | `crates/htui-orch/src/engine.rs:20119` `close_out_preview_counts_what_close_out_writes` | engine unit test over MemStore |
| Tests (TUI unit) | `crates/htui/src/ui/tabs/backlog/detail/runs.rs:4410` `the_chosen_resolution_is_what_close_out_sends` | drive keys, assert the sent `Command` |
| Tests (TUI e2e) | `crates/htui/tests/it/backlog_runs.rs:126` `the_close_out_asks_inside_the_detail_pane` | full app, insta snapshot of the modal |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-core/src/store/traits.rs` | UPDATE | `close_out` signature + doc; two sentence fns | T1 |
| `crates/htui-core/src/store/worker.rs` | UPDATE | `WorkerStore::close_out` signature; `WorkerStore::links`; MemStore impls | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE | `State::close_out` link rule + upsert; `MemStore::close_out`; unit-test call sites | T1 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | new cases + `CASES`; existing call sites pass `None` | T1 |
| `crates/htui-store/src/pg/write.rs` | UPDATE | `PgStore::close_out` link rule, lock, upsert | T1 |
| `crates/htui-store/src/worker.rs` | UPDATE | PgStore/Writer `WorkerStore` impls: `close_out`, `links` | T1 |
| `crates/htui-store/src/writer.rs` | UPDATE | `Writer::close_out` forwards `names` | T1 |
| `crates/htui-store/.sqlx/*.json` | CREATE (if needed) | new queries only (D3) | T1 |
| `crates/htui-store/tests/it/cache.rs` | UPDATE | call site `None` | T1 |
| `crates/htui-store/tests/it/pg_criteria.rs` | UPDATE | race: close-out naming a canonical vs an agent proposing another | T1 |
| `crates/htui-agent/src/conformance.rs` | UPDATE | `UsageSpy::close_out` forwards `names` | T1 |
| `crates/htui-agent/tests/it/recorder.rs` | UPDATE | `SpyStore::close_out` forwards `names` | T1 |
| `crates/htui-worker/tests/it/auto_queue.rs` | UPDATE | two call sites `None` | T1 |
| `crates/htui/tests/it/runs_pg.rs` | UPDATE | two direct `close_out` calls `None` (T1); three `Command::CloseOut` literals `names: None` (T2) | T1, T2 |
| `crates/htui-orch/src/engine.rs` | UPDATE | T1: call passes `None`; T2: resolve `names`, preview links, unit tests | T1, T2 |
| `crates/htui-orch/src/command.rs` | UPDATE | `Command::CloseOut.names`; its tests' literals | T2 |
| `crates/htui-orch/src/closeout.rs` | UPDATE | `Preview` fields; `preview` argument; tests | T2 |
| `crates/htui-orch/src/conformance.rs` | UPDATE | new orch case + `CASES` | T2 |
| `crates/htui/src/run_worker.rs` | UPDATE | two `Command::CloseOut` literals | T2 |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | T2: send `names: None` so it compiles; T3: `Naming` stage, summary text, tests | T2, T3 |
| `crates/htui/tests/it/backlog_runs.rs` + snapshots | UPDATE | e2e: a duplicate close-out asks for and records the canonical | T3 |

## Tasks

The tasks run in series, T1 → T2 → T3, because their file sets overlap: `engine.rs` and `runs_pg.rs`
in T1/T2, `runs.rs` in T2/T3. T1 is the foundation and gets a verifier.

### Task 1: The store records or requires the link at close-out
- **Action** (tests first):
  - **Conformance cases on both stores**, registered in `CASES`:
    - `close_out_duplicate_needs_or_records_its_canonical`:
      - Unnamed with no link → `close_out_needs_a_link`, with the row and documents unchanged.
      - Naming B → closed, and a live `item -duplicates-> B` with a NULL proposer.
      - On a second item: a live agent-proposed `duplicates` to B, unnamed → closes, and the link keeps
        its proposer.
      - Naming C with a live link to B → `second_canonical`.
      - Naming B again with a live link to B → closes, idempotent.
      - Naming B over a tombstoned link to B → it revives with a NULL proposer.
    - `close_out_superseded_needs_or_records_its_supersessor`: the mirror of the above for
      `named -supersedes-> item`, plus: a live `supersedes` from another item does not refuse naming a
      second one.
    - `close_out_names_only_for_duplicate_and_superseded`:
      - `withdrawn` naming B → `close_out_names_no_link`.
      - Naming the item itself → `self_link`.
      - An unknown id → `NotFound`.
      - An unreachable project → `link_outside_reach`.
    - `close_out_link_refusals_follow_the_law`: a closed item naming B gets `resolution_not_closable`,
      not a link refusal.
  - **`pg_criteria` race:** a close-out naming B and an agent `propose_link` of `item duplicates C`,
    concurrently. Exactly one canonical ends up live, and the loser gets `second_canonical`, never
    23505.
  - **Then implement D1–D6.** Every existing caller passes `None`. Run `cargo sqlx prepare` only if D3's
    reuse fails (scratch DB recipe in `docs/hr-sandbox.md`).
- **Mirror**: Patterns rows Naming, Errors, Link write (Pg/Mem), Close-out (Mem), Tests (store).
- **Validate**: `cargo nextest run -p htui-core --all-features`;
  `cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^(pg_conformance|pg_criteria|cache)::/)'`;
  `cargo build --workspace --all-features --tests`; `cargo sqlx prepare --check --workspace`.

### Task 2: The engine resolves the named key and the preview shows the standing link
- **Action** (tests first):
  - **Engine unit tests** (MemStore):
    - The preview's `duplicate_of` and `superseded_by` read live links, with a cross-project label
      qualified.
    - `CloseOut { names: Some("FIX-17") }` records the link.
    - An unknown key → `EngineError::Store(NotFound)`, with nothing written.
    - A qualified `slug:KEY` resolves.
  - **Orch conformance case**, registered in `CASES`: `close_out_records_the_named_canonical`.
  - **`closeout.rs` tests:** `preview` carries the new fields through.
  - **Then implement D7.** Update every `Command::CloseOut` literal (`command.rs` tests, `run_worker.rs`
    ×2, `runs_pg.rs` ×3, `runs.rs` production + tests) with `names: None`. `runs.rs`'s sender gets
    T3's value later.
- **Mirror**: Tests (orch), Tests (preview).
- **Validate**: `cargo nextest run -p htui-orch --all-features`;
  `cargo nextest run -p htui --all-features -E 'test(/run_worker::/) or (binary(it) and test(/^runs_pg::/))'`;
  `cargo build --workspace --all-features --tests`.

### Task 3: The Runs pane asks for the item when no link stands
- **Action** (tests first):
  - **`runs.rs` unit tests:**
    - `duplicate` with no live link: `y` enters `Naming`; typing `FIX-17`, Enter, then the item key and
      Enter sends `CloseOut { resolution: Duplicate, names: Some("FIX-17") }`.
    - With a live `duplicate_of`: `y` goes straight to `Typed`, and the summary reads "· duplicate of
      FIX-17".
    - An empty submit in `Naming` errors and stays.
    - `Esc` in `Naming` returns to browse.
    - `superseded` behaves the same way.
    - Update `the_chosen_resolution_is_what_close_out_sends` (`:4410`) and the cases at `:7447`/`:7726`
      that pick `Duplicate`.
  - **e2e** in `backlog_runs.rs`: close a fixture item as `duplicate`, naming another. The modal
    snapshot shows the naming line, and the store holds the link afterwards.
  - **Then implement D8.**
- **Mirror**: Tests (TUI unit), Tests (TUI e2e).
- **Validate**: `cargo nextest run -p htui --all-features -E 'test(/detail::runs::/) or (binary(it) and test(/^backlog_runs::/))'`;
  `cargo insta test -p htui --all-features` (review every moved snapshot; expected: only the new ones).

## Validation

```bash
bash scripts/gate.sh                                   # full gate (main thread, once for the lane)
cargo clippy --workspace -- -D warnings                # featureless clippy (memory: featureless gate)
cargo sqlx prepare --check --workspace
cargo insta test --workspace --all-features            # full insta run; review every moved snapshot
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `item_link.proposed_by_step_id` is nullable; NULL means a non-agent writer | TRUE | `migrations/0001_init.sql:361` `proposed_by_step_id UUID, -- NULL = importer`; `model/link.rs` `ItemLink` doc "None means a human or the importer" |
| `close_out` has 5 `WriteStore` impls and 3 `WorkerStore` impls | TRUE | `impl … WriteStore for`: `mem.rs:10647`, `pg/write.rs:2048`, `writer.rs:375`, `htui-agent/src/conformance.rs:781`, `recorder.rs:469`; `impl … WorkerStore for`: `core/store/worker.rs:778`, `htui-store/src/worker.rs:117, 640` |
| The engine's store bound is `WorkerStore`, which has `item_by_key` but no `links` | TRUE | `engine.rs:516` `S: htui_core::store::WorkerStore`; `core/store/worker.rs:511` `item_by_key`; no `fn links(` in `core/store/worker.rs` |
| `item_by_key` resolves a qualified `slug:KEY` | TRUE | `mem.rs:9191` `split_qualified`; conformance `item_by_key_resolves_a_qualified_key` (`:31249`) |
| Close-out surfaces: only the Runs pane sends `CloseOut`; the worker relays it with `..` | TRUE | `Command::CloseOut` constructors: `runs.rs:1213` (production), test files, `run_worker.rs` tests; `htui-worker/src/runtime.rs:2845` and `views.rs:78` match `{ item, .. }` / `{ .. }` |
| `Command` and `Preview` cross no serialisation boundary | TRUE | `command.rs:56` `#[derive(Debug, Clone, PartialEq, Eq)]`; `closeout.rs:17` same, no serde; the worker boxes `Preview` in-process (`views.rs:107`) |
| No existing store test closes as `duplicate`/`superseded` (so D2 breaks none) | TRUE | `Resolution::(Duplicate\|Superseded)` hits outside `model/item.rs`: only `runs.rs` (TUI tests, T3 updates them), `label.rs`, `requirements/detail.rs` (display); `Resolution::ALL` loops in conformance (`:15407, :15488`) only exercise *refused* pairs, which the law refuses before D2 runs |
| The law runs before any link check, so existing refusal sentences hold | TRUE by D2 | `close_out_resolution_law` asserts `resolution_not_closable` for every non-closable pair; D2 places the link rule after `closes_from` |
| `propose_link`'s duplicates lock key and query texts exist to reuse | TRUE | `pg/write.rs:10103` lock key `item_link:duplicates:{from}`, `SELECT pg_advisory_xact_lock(hashtextextended($1, 0))`, second-canonical `SELECT … LIMIT 1` |
| sqlx's `query_as!` accepts `Option<Uuid>` for the reused INSERT's `$4` | UNVERIFIED — D3 makes it a try with a fallback | compile probe left to T1; the fallback is a new query + `cargo sqlx prepare` |
| No migration needed: both kinds are in the M1 CHECK | TRUE | `0030_link_kinds` `chk_item_link_kind` lists seven kinds (M1 plan D8) |
| Tasks are serial; overlaps are `engine.rs`, `runs_pg.rs` (T1/T2) and `runs.rs` (T2/T3) | TRUE | Files table; marked serial, so no parallel claim to check |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The signature change misses a call site in a featureless build | Medium | `cargo build --workspace --all-features --tests` in T1 plus the featureless clippy in validation |
| An incoming `supersedes` is withdrawn between the check and the commit (D4) | Low | accepted; MOD-155 makes `supersedes` pending |
| A key typed in `Naming` is a typo | Medium | the engine answers `NotFound`, nothing is written, and the person retries |
| `.sqlx` drift | Medium | `cargo sqlx prepare --check` in T1 and validation |
| Close-out snapshot churn | Low | the summary text changes only for `duplicate`/`superseded`; full insta run |
| Sandbox gate flakes (live Qdrant, scheduling) | High | the host Postgres/Qdrant gate is owed (memory) |

## Acceptance

- [ ] All tasks complete, tests written before code
- [ ] Validation passes (sandbox), host Postgres/Qdrant gate noted as owed
- [ ] Patterns mirrored, not reinvented
- [ ] PRD M2 row → complete; HANDOFF phase note (P1); PRD open question 3 ticked with D10
