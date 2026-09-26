# Plan: MOD-7 milestone 3 — a mismatch is refused by name

> **Status: confirmed** (2026-09-26; maintainer adopted every recommended answer, OQ-20..OQ-25), fact-checked 2026-09-26. Every tree fact this plan relies on is
> listed under "Claims to verify"; "Verified claims" holds the verdicts, and amendments are marked
> inline with "(amended at fact-check)".
>
> **Fact-check:** 58 claims — 41 verified, 15 amended, 2 falsified (#14 `Claim: Copy` is relied on
> in `pg_criteria.rs`; #56 T1 did not compile on the base) — both fixed in the design (T0 gains
> `pg_criteria.rs`, T2 gains `command.rs`); both waves independent after the fixes.

**Source**: `.claude/prds/mod-7-box-registry.prd.md`, milestone 3 (Delivery Milestones table, row 3):
"`R-ORCH-10` at enqueue and at claim, the engine's box-tag seam, criterion 14's capability half as
its own conformance case." Scope bullet "Capability refusal (`R-ORCH-10`)": "ANA-2 §4.10 as
designed: at enqueue, a missing tag blocks the item with a note naming exactly the missing tags and
writes no `run`; at claim, inside the admission transaction, the run fails with `missing tags: a,
b`. The engine reaches box tags through a seam it does not have today. Criterion 14's capability
half gets its own conformance case." Success-metric row "Capability refusal" ("no `run` row,
`item.status = 'blocked'`, note body exactly the missing tags; `Unblock` reopens … the ANA-2
claim-time case fails the run with `missing tags: …`"). Risk row "Tag check at claim races a
re-probe" ("ANA-2 §4.10 puts the check inside the admission transaction; the conformance case
covers the claim path"). Design authority: `docs/ANA-2.md` §4.10 (capability check), §12 criterion
14, and the PRD's gate decisions **PRD D0–D7**, which are binding and not reopened here (none of
them is specific to this milestone; D0 — MOD-40 is not a dependency — is the one that applies).

**Requirements**: `R-ORCH-10` (`docs/REQUIREMENTS.md:234-235`: "a run is refused when the item's
required tags are not a subset of the box's tags, listing the missing tags. Auto mode filters the
queue the same way"); `R-BOX-3` (`:74-76`: probed and hand-declared tags); ANA-2 invariant 7 (a
refusal is a row a human can read). The auto-mode half of `R-ORCH-10` is MOD-12's (PRD out of scope:
"`ready_items` already filters by tags; wiring it into a queue is MOD-12's").

**Complexity**: Medium-small. **No migration**: `item.required_tags` (`0001_init.sql:322`),
`box.probed_tags`/`box.declared_tags` and `run.failure` (`0001_init.sql:462`, plain `TEXT`) all
exist. One new `Claim` variant; one new check inside `claim_run` in both stores (three new
`query!` statements on Postgres); one new `GraphSource` method across four implementations; one new
`RunFailure` and one new `EngineError` variant; two store conformance cases, two `htui-orch`
conformance cases and one Postgres end-to-end test through the worker. No UI change, no new
`StoreRequest`, no snapshot.

**Routing**: routed as **plan** by `/handoff-run MOD-7` (the PRD, accepted 2026-09-25, and its
milestone table exist; milestones 1 and 2 are complete). **Staffing: Opus 5.5 for every step —
plan, fact-check, architect, implementers, verifiers and reviewer (`rust-reviewer`); Fable is not
used (maintainer standing instruction).** Ultracode for the implementers only, one workflow per
task, verify fan-out per round; the architect and the reviewer stay plain agents.

**Numbering**: milestone 1 used D1–D38, R-1–R-19, OQ-1–OQ-12; milestone 2's plan used D39–D53,
R-20–R-28, OQ-13–OQ-19, and its blueprint D54–D74 and R-29–R-32. This plan's decisions are
**D75…D89**, risks start at **R-33**, open questions at **OQ-20**. Tasks restart at **T0** and are
always cited as "milestone 3 T*n*" outside this file. The PRD's gate decisions are cited as **PRD
D0…PRD D7**; MOD-4's plan decisions as **MOD-4 D*n***.

**Base**: `main` at `98e6d2f` (after PR #8, which carries MOD-9 milestone 2 and the merge of MOD-7
milestone 2). Every line number below is the file's at `98e6d2f`, pre-edit.

**Graphify note**: `graphify-out/` does not exist in this checkout, so nothing here was read from
it. Tree facts were located through the Gortex index (symbol search, symbol source, usages,
implementations) and cited at the line Gortex reported; the fact-check pass re-reads each at its
line in the file, because the index has been stale by a few lines before (milestone 2's note).

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked.

- [x] **OQ-20 — The note's body.** ANA-2 disagrees with itself: §4.10 says the `item_note` "names
      the box and the missing tags", §12 criterion 14 says its body "is exactly the missing tags",
      and the PRD metric repeats criterion 14. **Default adopted (D78):** the body is the same
      sentence the claim-time path writes into `run.failure`, `missing tags: a, b` (sorted by byte,
      deduplicated, `", "`-joined), and the box is named structurally by the note's own
      `item_note.box_id`, which `Engine::note` already fills with the engine's box
      (`crates/htui-orch/src/engine.rs:2954-2974`). One sentence in both places, and the rung-4 note
      already follows this shape (its body is `RunFailure`'s `Display`, `engine.rs:690-706`).
      **Alternative:** a literal body of only the tags (`a, b`), or a body that adds the hostname
      (`missing tags on DESKTOP-HTUI: a, b`), which needs a box read the engine does not do today.
- [x] **OQ-21 — Where the claim-time failure is written.** ANA-2 §4.10 puts the check "inside the
      admission transaction". **Default adopted (D80):** `claim_run` itself, in the transaction that
      already holds the run and box rows `FOR UPDATE`, moves the run `queued -> failed` with
      `failure = "missing tags: a, b"` and the item `queued -> blocked`, and answers the new
      `Claim::MissingTags { missing }`; the engine then writes the note. This makes `MissingTags`
      the one `Claim` verdict that writes, and `Claim`'s `#[must_use]` text ("a refused claim wrote
      nothing") is reworded. **Alternative:** `claim_run` writes nothing and the engine follows up
      with `finish_run` and `transition`. Rejected as default because between the two transactions
      another process on the same box could claim the run after a re-probe added the tag, and
      `finish_run` would then fail a `running` run (`running -> failed` is a legal move,
      `crates/htui-core/src/model/run.rs:60-70`).
- [x] **OQ-22 — Order of the checks.** **Default adopted (D79, D81):** at enqueue the tag check
      runs **before** `graph::resolve`, so an item that both lacks tags and has no candidate agent
      is refused for the tags (the cheaper, box-level fact). At claim it runs after the
      `NotClaimable` test and **before** `SlotFull` and `Overlaps`, so a permanent refusal wins over
      a transient one (a transient refusal re-queues the run and it would meet the tag refusal on
      the next claim anyway). **Alternative:** after resolve at enqueue, and last at claim.
- [x] **OQ-23 — An item at `failed`.** `create_run` accepts `open | failed`, but the item law has
      no `failed -> blocked` edge (`crates/htui-core/src/model/item.rs:52-66`). **Default adopted
      (D82):** the enqueue refusal mirrors rung 4 exactly: the `open -> blocked` compare-and-set
      answers `Ok(false)` for a `failed` item, which stays `failed`, and the note is still written
      (`engine.rs:685-689`, the rung-4 doc says the same; amended at fact-check). Criterion 14 is about an `open` item and
      is proved on one. **Alternative:** widen the item law with `failed -> blocked`, which is an
      ANA-2 §4.3 amendment and out of this milestone's scope.
- [x] **OQ-24 — Showing the missing tags before a run is attempted.** `MemStore::missing_tags`'s
      doc says it is "what the Backlog renders beside an item it cannot start here"
      (`crates/htui-core/src/store/mem.rs:672-673`), and nothing renders it. **Default adopted
      (D88):** not in this milestone; the PRD scopes milestone 3 to the refusal. The main thread
      opens a small follow-up item if the maintainer wants the Backlog hint. **Alternative:** add a
      Backlog detail line in this milestone (a `StoreRequest`, a section change and snapshots —
      roughly doubling the milestone).
- [x] **OQ-25 — Syntax of a required tag.** Declared tags are validated
      (`canonical_declared_tags`, `crates/htui-core/src/model/box_.rs:184`, over `is_declared_tag`,
      `box_.rs:167`: `[a-z0-9_-]{1,64}` whose first character is a letter or digit; amended at
      fact-check),
      probed tags come from the seed spec, but `item.required_tags` is unconstrained. An item that
      requires `Rust` is refused for `Rust` on a box tagged `rust`. **Default adopted (D76):** no
      normalisation and no validation here; the comparison is exact bytes, and the refusal names
      the offending tag, which is readable. **Alternative:** validate `required_tags` with the
      declared-tag rule in `update_item`/`create_item` (a store constraint on existing rows, better
      as its own item).

---

## Summary

The capability refusal exists in the store and nowhere else. `MemStore::missing_tags`
(`crates/htui-core/src/store/mem.rs:678-696`), `PgStore::missing_tags`
(`crates/htui-store/src/pg/read.rs:1919-1962`) and `Backend::missing_tags`
(`crates/htui-store/src/backend.rs:561-567`) compute `required_tags − (probed_tags ∪ declared_tags)`
in byte order and are called only by tests (`mem.rs:8966-8976`,
`crates/htui-store/tests/pg_criteria.rs:3247-3280`). They are inherent methods, so an engine generic
over `S: ReadStore + WriteStore` cannot reach them. `claim_run` checks the slot and the overlap
rules only (`mem.rs:3550-3638`, `crates/htui-store/src/pg/write.rs:2990-3132`); `Claim` has no tag
verdict (`crates/htui-core/src/model/overlap.rs:126-148`) and `RunFailure` no missing-tags case
(`crates/htui-orch/src/status.rs:39-100`). MOD-4 proved criterion 14's `Unblock` half through the
rung-4 no-candidate refusal instead (`crates/htui-orch/src/conformance.rs:5200-5248`,
`unblock_opens_a_blocked_item_with_no_run`: `with_candidates("prd", Vec::new())`, `StartRun` refused
before a run row exists, item `blocked`, `Unblock` answers `UnblockCase::Reopen`, and a later
`StartRun` walks).

**At claim (T0).** Both stores' `claim_run` gain the tag check inside the admission transaction:
after the `NotClaimable` test, before `SlotFull`. A run whose item's `required_tags` are not a subset
of the claiming box's `probed_tags ∪ declared_tags` is moved `queued -> failed` with `run.failure =
"missing tags: a, b"`, its item `queued -> blocked`, and `claim_run` answers
`Claim::MissingTags { missing }`. The sentence is one function in `htui-core` so both stores and
`htui-orch` write the same bytes.

**At enqueue (T1).** `GraphSource` (`crates/htui-orch/src/graph.rs`, the trait whose whole purpose
is "what `htui-orch` needs from a store … that no `ReadStore` method answers") gains
`missing_tags(item, box_id)`. Each implementation delegates to the inherent read of the same name,
as the other six methods already do. `Engine::enqueue` asks it before `graph::resolve`; a non-empty
answer takes the rung-4 path: `open -> blocked`, a note `missing tags: a, b`, no `create_run`, and
the new `EngineError::MissingTags`. Criterion 14's capability half becomes its own `htui-orch`
conformance case, whose round trip ends with the box declaring the tags (milestone 2's
`WriteStore::edit_box`) and a fresh `StartRun` walking.

**The engine side of the claim (T2).** `Engine::claim` maps `Claim::MissingTags` to the note and
`EngineError::MissingTags` rather than `EngineError::ClaimRefused`, so the run worker does not
re-queue a failed run (`crates/htui/src/run_worker.rs:1944`, `:2162` re-queue only on
`ClaimRefused`). A second `htui-orch` case proves the claim-time path.

**Postgres end to end (T3).** One test in `crates/htui/tests/runs_pg.rs` drives criterion 14 through
the worker on Postgres, so `run_worker::BackendGraphs`'s new method and `PgStore`'s claim check are
exercised by the production wiring.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D75 | **The seam is `GraphSource::missing_tags`.** `async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>>` is added to `htui_orch::graph::GraphSource` (declared at `crates/htui-orch/src/graph.rs:57`, after the `#[allow(async_fn_in_trait)]` at `:56`, doc `:33-55`; amended at fact-check; T1 may also correct the doc's stale "not a fifth method" at `:49`, since the trait already has six), documented as `R-ORCH-10`'s read, "byte order, deduplicated; `NotFound` for an unknown item or box, the item first". Every implementation delegates to the inherent read of the same name: `impl GraphSource for MemStore` (`crates/htui-orch/src/fake.rs:844`) calls `MemStore::missing_tags`; `FakeGraphSource` (`fake.rs:963`) calls `GraphSource::missing_tags(self.store, …)` like its `agent` method; `run_worker::BackendGraphs` (`crates/htui/src/run_worker.rs:2366`) calls `Backend::missing_tags`; the test-only `TestSource` (`graph.rs:797`) delegates to its store the same way. The unit test that calls every `impl GraphSource for MemStore` method through the trait (doc at `fake.rs:2117`, "delegates to the six inherent reads") gains the seventh. | The trait's own doc names exactly this situation: inherent reads the engine cannot reach through `S`, and "`htui-store` never learns that an engine exists" (`graph.rs:33-47`). It is the MOD-4 precedent for reaching `Backend` from the engine (MOD-4 D155), just as `Backend::repo_paths(box)` reaches the production isolator. Moving `missing_tags` onto `ReadStore` instead would force the SQLite mirror to implement it (the mirror has `box` but runs no orchestration reads), and onto `WriteStore` would add a method to five implementations for a read. A new trait would add a ninth generic parameter to `Engine<'a, S, G, I, V, C, A, K>` (`engine.rs:459`). |
| D76 | **Tag semantics.** Effective tags are `box.probed_tags ∪ box.declared_tags`; the only requirement source is `item.required_tags` (`crates/htui-core/src/model/item.rs:139-140`; `step_graph_phase` carries no tags, `crates/htui-core/src/model/kind.rs:177-212`). Comparison is exact bytes with no case folding or trimming (OQ-25). The missing set is sorted by byte and deduplicated, which is what both existing reads already return (`mem.rs:692-693`: `sort_by` on `as_bytes`, `dedup`; `pg/read.rs:1922-1926`: `SELECT DISTINCT t COLLATE "C" … ORDER BY 1`; amended at fact-check). An item with empty `required_tags` is never refused, and a chat run (`run.item_id IS NULL`) has no tags to check. | ANA-2 §4.10's predicate is `i.required_tags <@ (b.probed_tags \|\| b.declared_tags)` and its message the `EXCEPT` of the two; the shipped `t <> ALL (…)` with `DISTINCT` is the same set. `State::box_capabilities` (`mem.rs:3346-3357`) already documents "`probed_tags ∪ declared_tags` (`R-ORCH-10`)". |
| D77 | **One sentence, in `htui-core`.** `pub fn missing_tags_failure(missing: &[String]) -> String` in `crates/htui-core/src/model/overlap.rs` beside `Claim`, re-exported from `model/mod.rs` with it, answers `format!("missing tags: {}", missing.join(", "))`. Both stores write it into `run.failure`; `RunFailure::MissingTags`'s `Display` and `EngineError::MissingTags`'s `#[error]` (T1 writes both as literals, because T1 compiles against the base `htui-core`; T2 switches both to call this function — amended at fact-check) and the notes use it. A unit test pins the bytes: `["a", "b"]` → `missing tags: a, b`. | ANA-2 §4.10 fixes the text (`run.failure = "missing tags: a, b"`). The store cannot depend on `htui-orch`, so the one source of the bytes lives in the crate both depend on — the reason the finish-run sentences live in `traits.rs` (`crates/htui-core/src/store/traits.rs:1151-1157` docs). |
| D78 | **The note** (OQ-20). Body `missing tags: a, b` (D77); `box_id` the engine's box, `via_step_id` NULL, written by `Engine::note` (`engine.rs:2954-2974`). At enqueue it follows the `open -> blocked` compare-and-set, as rung 4 does (`engine.rs:690-706`); at claim it follows `claim_run`'s transaction. | Criterion 14 asks for the tags exactly; §4.10's "names the box" is met by the row's `box_id`. |
| D79 | **The enqueue refusal** (`Engine::enqueue`, `engine.rs:596-641`). After `self.item(item)` and **before** `graph::resolve` (OQ-22), and only when the item is `open` or `failed` (the statuses `create_run` accepts, comment at `engine.rs:623-625`, amended at fact-check; any other status falls through to today's refusal from `create_run`, so no spurious note is written for an item that is already queued or running): `let missing = self.parts.graphs.missing_tags(item.id, self.parts.box_id).await?;` and, when non-empty, a new private `refuse_missing_tags(&item, missing)` in the shape of `refuse_rung_four`: `transition(item.id, Status::Open, Status::Blocked)` (its `Ok(false)` on a `failed` item is accepted, D82), then the note, then `Err(EngineError::MissingTags { item: item.id, run: None, missing })`. No `run` row is written. | ANA-2 §4.10: "A queue-time refusal writes no `run` row, so there is no orphan run for a human to cancel." Rung 4 is the proven shape (criterion 14's `Unblock` half, `conformance.rs:5200-5248`); `Unblock`'s `Reopen` case applies unchanged (`crates/htui-orch/src/command.rs:1135-1160`: `(Status::Blocked, _, None) => Reopen`). |
| D80 | **The claim refusal writes, inside the admission transaction** (OQ-21). `Claim` gains `MissingTags { missing: Vec<String> }`, with `Display` equal to D77's sentence. Because the payload is a `Vec`, `Claim` loses `Copy` (keeps `Debug, Clone, PartialEq, Eq`). **One caller copies it** (amended at fact-check; claim 14 was falsified): `crates/htui-store/tests/pg_criteria.rs:694` (`let refusal = if one.is_admitted() { two } else { one };`) moves both verdicts and `:709` then calls `one.is_admitted()`, which is E0382 without `Copy` (probed on rustc 1.98.1). T0 fixes it by borrowing (`&two`/`&one`) and comparing against `&Claim::SlotFull { .. }`. `#[must_use]`'s text becomes "a refused claim must be acted on; only `MissingTags` wrote anything". Every doc that says a refusal writes nothing is reworded to except `MissingTags`: `WriteStore::claim_run`'s (`traits.rs:840-841`, declaration `:851-858`), `State::claim_run`'s (`mem.rs:3549`) and `PgStore::claim_run`'s (`pg/write.rs:2988-2989`) (amended at fact-check); the trait doc also gains the verdict and the three writes. **`MemStore`** (`State::claim_run`, `mem.rs:3550-3638`): after the `NotClaimable` return (`:3566-3568`) and before the slot count, compute the item's missing tags from `box_capabilities`; when non-empty, set the run's `status = Failed`, `failure = Some(sentence)`, `finished_at = Some(at)`, `updated_at = now` (leaving `executing_box_id`, `started_at` and the lease untouched), then `self.transition(item, Status::Queued, Status::Blocked, now)` when the item is `queued` (a stale item status is not a refusal, as the admitted branch says at `:3628-3636`), and answer the verdict. **`PgStore`** (`pg/write.rs:2990-3132`): after `NotClaimable` (`:3030-3032`) and before the limit read, one `query_scalar!` for the missing tags joined through the run — `SELECT DISTINCT t COLLATE "C" AS "tag!" FROM run r JOIN item i ON i.id = r.item_id CROSS JOIN box b, UNNEST(i.required_tags) t WHERE r.id = $1 AND b.id = $2 AND t <> ALL (b.probed_tags \|\| b.declared_tags) ORDER BY 1` — then, when non-empty, `UPDATE run SET status = 'failed', failure = $2, finished_at = COALESCE(finished_at, $3) WHERE id = $1 AND status = 'queued'` and `UPDATE item SET status = 'blocked' WHERE id = (SELECT item_id FROM run WHERE id = $1) AND status = 'queued'`, `tx.commit()`, and the verdict. The run row is locked `FOR UPDATE` at `:3000-3016` and the box row at `:3018-3028` (amended at fact-check), so a concurrent `record_box_probe` or `edit_box` waits for the claim and the check cannot race a re-probe. **Constraint to respect** (amended at fact-check): `ck_run_graph_snapshot` (`0003_orchestration.sql:47-48`, added `NOT VALID`, so it still checks new rows and every `UPDATE`) refuses the `UPDATE run SET status = 'failed'` on a `kind = 'graph'` run whose `graph_snapshot` is NULL. Runs from `create_run` always carry a snapshot, so production is unaffected; no test may fail a snapshot-less seeded run (for example `RUN_2`) through `claim_run` on Postgres. | ANA-2 §4.10: "a claim-time refusal fails the run row that already exists with `run.failure = "missing tags: a, b"`" and the item is `blocked`. `finish_run_item_mirror(Failed, Queued)` answers `None` (`traits.rs:1599-1608`), so reusing `finish_run` would leave the item `queued`; the item move is therefore explicit. `queued -> failed` (`run.rs:60-70`) and `queued -> blocked` (`item.rs:52-66`) are both legal moves. |
| D81 | **Claim order** (OQ-22): run lookup (`NotFound "run"`), box lookup (`NotFound "box"`), `NotClaimable`, **`MissingTags`**, `SlotFull`, `Overlaps`, admit. A `NotClaimable` run with missing tags answers `NotClaimable` and writes nothing. | A permanent refusal before transient ones; the lookups keep their documented order (`traits.rs` "the run looked up first"). |
| D82 | **A `failed` item** (OQ-23) is refused at enqueue with the note and stays `failed`; `Unblock` does not apply to it (`unblock_enabled` answers `NotBlocked`), and a later `StartRun` after the tags exist runs it, since `create_run` accepts `failed`. At claim the item is `queued` by construction (`create_run` moved it), so it always goes `blocked`. | Mirrors rung 4 (`engine.rs:685-689`; amended at fact-check); no item-law change. |
| D83 | **`RunFailure::MissingTags(Vec<String>)`** in `crates/htui-orch/src/status.rs` (enum `:39-100`, `Display` `:102-136`), rendering D77's sentence; `run_failure_display_is_ana2s_bytes` (`status.rs:405`) gains its row. It is the typed reason the engine builds the note from, as `NoCandidateAgent` is for rung 4. | The note and `run.failure` read the same (D78). |
| D84 | **`EngineError::MissingTags { item: ItemId, run: Option<RunId>, missing: Vec<String> }`** in `crates/htui-orch/src/command.rs` (enum `:310-596`), `#[error]` = `item {item}: missing tags: a, b`, documented as `R-ORCH-10`'s refusal, `run` `None` at enqueue and `Some` at claim. **In T1 the sentence is a literal** (`"item {item}: missing tags: {}"` over `missing.join(", ")`, the same literal as `RunFailure::MissingTags`'s `Display`), because T0's `missing_tags_failure` does not exist on the base T1 builds on; **T2 switches it** to `htui_core::model::missing_tags_failure` (amended at fact-check; claim 56 was falsified). `the_named_refusals_say_what_went_wrong` (fn `command.rs:1614`, which builds `ClaimRefused` at `:1626` and `:1637`) gains both shapes. It is **not** `ClaimRefused`: `ClaimRefused`'s doc says "the run stays `queued`" (`:375-383`; amended at fact-check), and the run worker re-queues exactly `ClaimRefused` (`run_worker.rs:1944`, `:2162`); a `MissingTags` falls into the worker's generic `Some(Err(err)) => ctx.refuse(err.to_string())` arm, so the refusal reaches the status line and nothing is re-queued. No `crates/htui` source changes for this (T1 touches `run_worker.rs` only for `BackendGraphs`). | Keeps the worker's retry loop about transient refusals only. |
| D85 | **`Engine::claim`** (`engine.rs:666-683`): on `Claim::MissingTags { missing }` it reads the run's `item_id` (`ReadStore::run`), writes the note (D78) and returns `EngineError::MissingTags { item, run: Some(run), missing }`; every other non-admitted verdict stays `ClaimRefused`. The `claim` doc and the comment at `:669-671` (amended at fact-check) name the new verdict. | The note stays out of the store transaction because `claim_run` is not given the engine's user, and the note is the engine's record of its own refusal (D78). Rung 4 already writes its note after its transition in the same way. |
| D86 | **Criterion 14's capability half, its own case** (T1): `a_capability_refusal_writes_no_run_and_unblock_reopens` in `crates/htui-orch/src/conformance.rs`, beside `no_candidate_agent_blocks_the_item` in `CASES`. Over the demo fixture, whose box is probed `rust, msvc, cmake` and declared `gpu` (`crates/htui-core/src/fixtures.rs:458-480`): `free_feat_3`; `update_item` sets `HTUI_FEAT_3`'s `required_tags` to `["rust", "vulkan", "docker"]`; `StartRun` is refused with `EngineError::MissingTags { run: None, missing: ["docker", "vulkan"] }`; `store().runs(HTUI_FEAT_3)` is unchanged; the item is `blocked`; exactly one new note, body `missing tags: docker, vulkan`, `box_id = Some(ids::BOX)`; `Unblock` answers `UnblockCase::Reopen` and the item is `open`; `edit_box(ids::BOX, 0, BoxEdit { declared_tags: Some(["docker", "gpu", "vulkan"]), quirks: None })` is `Applied`; `StartRun` walks to `AwaitingApproval`, exactly as the MOD-4 case ends (`conformance.rs:5238-5246`). The MOD-4 case keeps its name and body; its doc (`:5197-5199`; amended at fact-check) and the `CASES` preamble (`:330-333`) are reworded from "criterion 14's `Unblock` half" to "rung 4's `Unblock` round trip; criterion 14 itself is `a_capability_refusal_writes_no_run_and_unblock_reopens`". | The PRD's hypothesis: "criterion 14 in full, with the capability half no longer proved by proxy". The same box after a declaration is "a box with those tags" (no trait method creates a second box, milestone 2 fact-check). |
| D87 | **The claim-time case** (T2): `a_capability_refusal_at_claim_fails_the_run_by_name`. Two items minted with overlapping `touched_paths` (`mint_feat`, `conformance.rs:3510`); the second is given `required_tags = ["gpu"]` (declared on the fixture box, so enqueue passes); the first `start`s and parks, holding its scope; `start_refused` on the second answers `Claim::Overlaps` and leaves its run `queued` (the shape of `overlapping_touched_paths_serialise`, `conformance.rs:3557`, whose checks at `:3571-3584` assert the refused run stays `queued`; amended at fact-check — `:3661-3718` never checks the `Overlaps`-refused run's status); `edit_box` clears the declared tags (the re-probe the PRD risk names); `orch.claim(run)` is refused with `EngineError::MissingTags { run: Some(run), missing: ["gpu"] }` even though the holder still overlaps (D81); the run is `failed` with `failure = "missing tags: gpu"`, `executing_box_id` and `started_at` NULL; the item is `blocked` with the note; `Unblock` reopens it (`Reopen`, since the failed run is not active). | ANA-2 §4.10's reason for the second check: "a box's `probed_tags` can change between the two". |
| D88 | **Not changed, on purpose:** no migration (next stays `0008`); no `StoreRequest`/`StoreReply` variant, no Settings or Backlog change, no snapshot (OQ-24); `ready_items` and auto mode (MOD-12); `required_tags` validation (OQ-25); the SQLite mirror (it mirrors the resulting `item`/`run` rows through its normal refresh); `box_probe` and the probe spec; `repo_box_path`, inference and excerpts (milestone 4), including the stale "no writer" comments at `engine.rs:4928`/`:5615` (the PRD's record corrections that belong to milestone 4); `docs/**`, `HANDOFF.md` and the PRD (the main thread records deviations). | The PRD's milestone 3 row and out-of-scope list. |
| D89 | **Postgres end to end** (T3): `a_capability_refusal_blocks_and_unblock_reopens_on_postgres` in `crates/htui/tests/runs_pg.rs`, over its `Stack` (`:235`) and `free_feat_3` (`:504`), in the shape of `unblock_follows_an_escalated_run_on_postgres` (`:940`): `PgStore::update_item` sets the tags, `Command::StartRun` through the worker puts `item …: missing tags: docker, vulkan` on the status line (`Stack::take_status`), `run_ids` is unchanged (`[ids::RUN_2]`, the seeded run `free_feat_3` cancels; `PgStore::runs` still returns it — amended at fact-check), the item is `blocked`, `Stack::notes` holds the sentence; `Command::Unblock`; `edit_box` declares the tags; `Stack::start` walks. The claim-time path on Postgres is T0's store conformance case over `PgStore`. The worker half of the claim-time path — that a `MissingTags` from `Engine::claim` is **not re-queued**, which lives in `crates/htui/src/run_worker.rs` and no `htui-orch` test can see — is T3's second test, `a_claim_time_refusal_is_not_requeued_on_postgres` (amended at fact-check): two items with overlapping `touched_paths`, the second requiring `gpu`; the first parks and holds its scope; the second's `StartRun` is refused `Overlaps` and joins the worker's queue; `edit_box` clears `gpu`; cancelling the first ends a task, so the worker's `claim_queued` retries the second; its run is then `failed` with `missing tags: gpu`, the item `blocked`, the status line carries the sentence, and a further task end leaves the run `failed` (nothing re-queued it). If `Stack`'s seed cannot give the two items a shared repository, the test is dropped and the property rests on the worker's match arms, which the reviewer checks. | Proves `BackendGraphs::missing_tags` and `PgStore::claim_run` in the production wiring; the PRD metric names both stores. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A refusal before any run row: transition, note, error | `Engine::refuse_rung_four` and its call in `enqueue` | `crates/htui-orch/src/engine.rs:685-706` (amended at fact-check), `:608-620` |
| The engine's note writer | `Engine::note` | `engine.rs:2954-2974` |
| A claim verdict mapped to an engine error | `Engine::claim` | `engine.rs:666-683` |
| A `GraphSource` method delegating to the inherent read | `impl GraphSource for MemStore`; `FakeGraphSource::agent`; `BackendGraphs` | `crates/htui-orch/src/fake.rs:844-892`, `:963-1010`; `crates/htui/src/run_worker.rs:2366` |
| The missing-tags read on each store | `MemStore::missing_tags`, `PgStore::missing_tags` | `crates/htui-core/src/store/mem.rs:678-696`; `crates/htui-store/src/pg/read.rs:1919-1962` |
| The admission transaction and its early returns | `State::claim_run`, `PgStore::claim_run` | `mem.rs:3550-3638`; `pg/write.rs:2990-3132` |
| Failing a run row in SQL | `PgStore::finish_run`'s `UPDATE run` | `pg/write.rs:4328-4341` (inside `:4283-4389`; amended at fact-check) |
| A typed failure and its pinned bytes | `RunFailure` and `run_failure_display_is_ana2s_bytes` | `crates/htui-orch/src/status.rs:39-136`, `:405` |
| A named engine refusal and its sentence test | `EngineError::ClaimRefused`, `the_named_refusals_say_what_went_wrong` | `crates/htui-orch/src/command.rs:375-383`, fn `:1614` (builds at `:1626`, `:1637`; amended at fact-check) |
| The rung-4 conformance case and the `Unblock` round trip | `no_candidate_agent_blocks_the_item`, `unblock_opens_a_blocked_item_with_no_run` | `crates/htui-orch/src/conformance.rs:2574-2620`, `:5200-5248` |
| A queued run left by a claim refusal, then `claim` | `overlapping_touched_paths_serialise`, `start_refused`, `mint_feat` | `conformance.rs:3557` (checks `:3571-3584`; amended at fact-check), `:3538-3551`, `:3510` |
| A store conformance case over `claim_run` | `claim_run_admits_one_and_refuses_the_second` | `crates/htui-core/src/store/conformance.rs:4158` |
| Changing the box's declared tags in a case | `edit_box_is_cas_on_edit_version` | `conformance.rs:5535` |
| A Postgres case through the worker, `Unblock` included | `unblock_follows_an_escalated_run_on_postgres` | `crates/htui/tests/runs_pg.rs:940` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/overlap.rs` | edit | T0 | `Claim::MissingTags`, drop `Copy`, `#[must_use]` text, `Display` arm; `missing_tags_failure` and its unit test (D77, D80) |
| `crates/htui-core/src/model/mod.rs` | edit | T0 | re-export `missing_tags_failure` beside `Claim` |
| `crates/htui-core/src/store/traits.rs` | edit | T0 | `WriteStore::claim_run`'s doc: the verdict, its order and its writes (D80, D81) |
| `crates/htui-core/src/store/mem.rs` | edit | T0 | `State::claim_run`'s tag check and writes (D80) |
| `crates/htui-core/src/store/conformance.rs` | edit | T0 | two cases, `CASES` 74 → 76 |
| `crates/htui-core/tests/mem_store.rs` | edit | T0 | pin 74 → 76 (`:36-37`) and its explanatory string |
| `crates/htui-store/src/pg/write.rs` | edit | T0 | `PgStore::claim_run`'s tag check and writes (D80) |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T0 | `EXPECTED_CASES` 74 → 76 (`:19`) |
| `crates/htui-store/tests/pg_criteria.rs` | edit | T0 | borrow the two verdicts at `:694` and `:709` (compare against `&Claim::SlotFull { .. }`), since `Claim` loses `Copy` (D80; amended at fact-check) |
| `crates/htui-store/.sqlx/` | regenerate | T0 | +3 statements (264 → 267; exact figure recorded at close) |
| `crates/htui-orch/src/graph.rs` | edit | T1 | `GraphSource::missing_tags` and its doc; `TestSource`'s implementation (D75) |
| `crates/htui-orch/src/fake.rs` | edit | T1 | `impl GraphSource for MemStore` and `FakeGraphSource` implementations; the through-the-trait unit test and its "six" doc (D75) |
| `crates/htui-orch/src/status.rs` | edit | T1, T2 | T1: `RunFailure::MissingTags` with a literal `Display` and its byte row; T2: `Display` calls `htui_core`'s sentence (D83) |
| `crates/htui-orch/src/command.rs` | edit | T1, T2 | T1: `EngineError::MissingTags` with a literal sentence and its sentence test (D84); T2: the sentence through `htui_core::model::missing_tags_failure` (amended at fact-check) |
| `crates/htui-orch/src/engine.rs` | edit | T1, T2 | T1: the enqueue check and `refuse_missing_tags` (D79); T2: `claim`'s mapping (D85) |
| `crates/htui-orch/src/conformance.rs` | edit | T1, T2 | T1: D86's case and its arm in `fn case` (`:513`), the MOD-4 case's doc, `CASES` 70 → 71, the pin test (fn `:5510`, `70` at `:5517`) and the count prose at `:288-292` and `:507`; T2: D87's case and arm, 71 → 72, the same prose (amended at fact-check) |
| `crates/htui-orch/tests/fake_conformance.rs` | edit | T1, T2 | `cases_len_is_seventy` → the new count (`:15-17`) |
| `crates/htui/src/run_worker.rs` | edit | T1 | `BackendGraphs::missing_tags` (D75) |
| `crates/htui/tests/runs_pg.rs` | edit | T3 | D89's two cases |

**Not touched, on purpose:** every migration, `cache_migrations/` and `crates/htui-store/tests/migrations.rs`
(no schema change); `crates/htui-store/src/pg/read.rs` and `crates/htui-store/src/backend.rs` (the
reads exist and are reused unchanged); every `htui-store` test file other than `pg_conformance.rs`
and `pg_criteria.rs` (the latter is touched only for the `Copy` fix, D80; amended at fact-check); `crates/htui-store/src/writer.rs`,
`crates/htui-agent/src/conformance.rs` and `crates/htui-agent/tests/recorder.rs` (their `claim_run`
forwards and no trait signature changes); `crates/htui-core/src/fixtures.rs` and the demo loader
(cases patch tags at run time); every snapshot; the Settings and Backlog UI; `docs/**`,
`HANDOFF.md`, the PRD.

## Tasks

**Order.** **Wave 1:** T0 and T1 in parallel, each in its own worktree. Merge T0, then T1, re-running
`htui-core`, `htui-store`, `htui-orch` and `htui` gates on the real tree after each merge. **Wave 2:**
T2 and T3 in parallel, each in its own worktree, both after Wave 1 is merged. Independence is decided
by intersecting the file sets below and by build coupling: a red or mid-edit commit in a dependency
crate stops every dependent crate compiling, which is why each parallel task runs in its own
worktree.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `crates/htui-core/src/model/overlap.rs`, `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/pg_criteria.rs` (amended at fact-check), `crates/htui-store/.sqlx/` | Wave 1, own worktree, parallel with T1 |
| T1 | `crates/htui-orch/src/graph.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/command.rs`, `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs`, `crates/htui/src/run_worker.rs` | Wave 1, own worktree, parallel with T0 |
| T2 | `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/command.rs` (amended at fact-check), `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs` | Wave 2, own worktree, after T0 and T1 merged; parallel with T3 |
| T3 | `crates/htui/tests/runs_pg.rs` | Wave 2, own worktree, after T0 and T1 merged; parallel with T2 |

**Intersections, checked.** T0 ∩ T1 = ∅: T0 owns only `htui-core` and `htui-store` paths, T1 only
`htui-orch` paths and `crates/htui/src/run_worker.rs`. T2 ∩ T3 = ∅: T2 owns `htui-orch` paths only,
T3 one `crates/htui` test file. T1 ∩ T2 = `{status.rs, command.rs, engine.rs, conformance.rs,
tests/fake_conformance.rs}`, five files — serial (Wave 1 before Wave 2; amended at fact-check).
T0 ∩ T2 = T0 ∩ T3 = T1 ∩ T3 = ∅.

**Hidden couplings checked.** `.sqlx/` moves in T0 only (T1–T3 add no `query!`; `runs_pg.rs` uses
runtime queries and `crates/htui` has no `.sqlx/`, claim to verify). No seed or fixture moves (every
case patches `required_tags` and `declared_tags` at run time). No snapshot moves (no UI change).
`tests/migrations.rs` does not move (no migration). The store `CASES` pins (`mem_store.rs:36-37`,
`pg_conformance.rs:19`) move in T0 only; the `htui-orch` `CASES` pins (`conformance.rs`'s
`cases_are_unique_and_seventy`, about `:5510`, and `tests/fake_conformance.rs:15-17`) move in T1
and T2, which are serial.

**Build coupling** (amended at fact-check). **T1 compiles against the base `htui-core`**: it uses
only the existing inherent `missing_tags` reads and `WriteStore::edit_box`/`update_item`, and it
spells the sentence as a literal in **both** places that render it, `RunFailure::MissingTags`'s
`Display` and `EngineError::MissingTags`'s `#[error]` (`"item {item}: missing tags: {}"` over
`missing.join(", ")`). T1 names nothing T0 adds; T2 switches both to
`htui_core::model::missing_tags_failure` (D77, D84). T0 changes `Claim` (a new variant, `Copy`
dropped). The base `htui-orch` still compiles against it, because `Engine::claim` only calls
`is_admitted()` and moves the verdict into `ClaimRefused`, and `Claim`'s one exhaustive match is its
own `Display` in `overlap.rs`. The one site that relied on `Copy`, `pg_criteria.rs:694`/`:709`, is
in T0's own file set and fixed there; T0's gate compiles every `htui-store` test target to prove it. Between the two merges an engine built
from T0 alone would report a claim-time `MissingTags` as `ClaimRefused` and the worker would re-queue
a run that is already `failed`; `reclaim` then drops it because it is not `queued`
(`run_worker.rs:1933-1935`; amended at fact-check). That window exists only on the real tree between two merges of one wave,
and T2 closes it. T2 needs T0's variant and function; T3 needs T0's claim check and T1's enqueue
check. **Both parallel markings stand.**

Every implementer prompt carries: PRD D0–D7 win over this plan where they disagree; read the tree,
not `graphify-out/` (it does not exist); the refusal is a note a human can read (invariant 7), never
a silent skip; **commit incrementally** (uncommitted subagent work does not survive the session, and
there is no stash on a shared tree), staging your own paths only; verify your gate with
`--test-threads=1` on the real tree after your merge.

### Task 0: the check inside the admission transaction (D76, D77, D80, D81)
- **Files**: as tabled.
- **Tests first.** `overlap.rs` unit tests: `missing_tags_failure_joins_in_the_given_order`
  (`["a", "b"]` → `missing tags: a, b`; one tag → `missing tags: a`) and
  `a_missing_tags_claim_displays_the_sentence`. Two store conformance cases appended to `CASES` after
  the last entry, with their `run_case` arms:
  `claim_run_fails_a_run_whose_item_needs_a_tag_the_box_lacks` (plant `required_tags = ["rust",
  "vulkan", "docker"]` on an item through `update_item`, create a queued run for it the way
  `claim_run_admits_one_and_refuses_the_second` does, `claim_run` answers
  `Claim::MissingTags { missing: ["docker", "vulkan"] }`; the run is `failed` with that failure,
  `finished_at = at`, `executing_box_id`, `started_at` and the lease untouched; the item is
  `blocked`; a second run of another item is still admitted, so no slot was consumed; after
  `edit_box` declares the tags, a fresh run of the same item — once the item is back at `open` —
  is `Admitted`) and `claim_run_checks_tags_after_claimability_and_before_the_slot` (a
  non-`queued` run with missing tags answers `NotClaimable` and writes nothing; with the box at
  `max_concurrent_items`, a run with missing tags answers `MissingTags`, not `SlotFull`). Pins 74 → 76.
  **Setup rule** (amended at fact-check): every run these cases fail through `claim_run` is made by
  `create_run` with a snapshot. None may be a snapshot-less seeded run such as `RUN_2`, because
  `ck_run_graph_snapshot` (`0003_orchestration.sql:47-48`) refuses the `failed` update of such a row
  on Postgres.
- **Action**: D77 and D80 on `MemStore` and `PgStore`; the three "a refusal writes nothing" docs
  (`traits.rs:840-841`, `mem.rs:3549`, `pg/write.rs:2988-2989`); the `claim_run` trait doc; the
  `Copy` fix in `pg_criteria.rs:694`/`:709` (amended at fact-check). `cargo sqlx
  prepare` against a scratch database migrated through `0007` (the compose `htui` database is empty;
  project memory).
- **Mirror**: `claim_run`'s existing branches; `finish_run`'s `UPDATE run`.
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo test -p
  htui-store --test pg_conformance --all-features -- --test-threads=1` with
  `HTUI_TEST_DATABASE_URL`; `cargo test -p htui-orch --all-features -- --test-threads=1` (still
  green: the base engine is unaffected, build coupling above); `cargo test -p htui-store
  --all-features --no-run` (every `htui-store` test target compiles without `Claim: Copy`; amended at
  fact-check); `cargo test -p htui-agent --all-features --no-run`; `cargo sqlx prepare --check`; `ls
  crates/htui-store/.sqlx | wc -l`. Commit boundary: one red commit, one green.

### Task 1: the seam, the enqueue refusal and criterion 14 (D75, D78, D79, D82, D83, D84, D86)
- **Files**: as tabled.
- **Tests first.** `status.rs`: `run_failure_display_is_ana2s_bytes` gains
  `RunFailure::MissingTags(vec!["a".into(), "b".into()])` → `missing tags: a, b`. `command.rs`:
  `the_named_refusals_say_what_went_wrong` gains `MissingTags { run: None, .. }` and `{ run: Some,
  .. }`. `fake.rs`: the through-the-trait unit test calls `missing_tags` (a demo item needing
  `cuda` answers `["cuda"]`, as `mem.rs:8966` does inherently). `engine.rs` unit test
  `enqueue_refuses_missing_tags_before_resolving` (a harness item whose phase also has no candidate
  is refused for the tags, not rung 4). Conformance: D86's
  `a_capability_refusal_writes_no_run_and_unblock_reopens`, added to `CASES` directly after
  `no_candidate_agent_blocks_the_item` with a comment naming ANA-2 §12 criterion 14, **and its arm
  in `fn case` (`conformance.rs:513`)**; the MOD-4 case's doc and the `CASES` preamble reworded
  (D86); both pins 70 → 71 (`conformance.rs:5517` in fn `:5510`, and
  `tests/fake_conformance.rs:15-17`); the count prose at `conformance.rs:288-292` ("Seventy…", the
  pin test's name, the "18 + 5 + 13 + 6 + 10 + 18" sum recounted) and at `:507` ("seventy-arm
  match") updated. Rename both pin tests to a count-free name (for example `cases_are_unique_and_counted`
  and `cases_len_is_pinned`) so later milestones do not rename them again (amended at fact-check).
- **Action**: D75 on all four implementations, D79, D82, D83 and D84 (both sentences as literals,
  so T1 names nothing T0 adds; amended at fact-check).
- **Mirror**: `refuse_rung_four`; `impl GraphSource for MemStore`.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; `cargo build -p htui
  --all-features` (BackendGraphs); `cargo test -p htui --all-features -- --test-threads=1`.

### Task 2: the engine side of the claim (D83, D85, D87)
- **Files**: as tabled.
- **Tests first.** `engine.rs` unit test `a_missing_tags_claim_is_not_a_claim_refused` (the error is
  `MissingTags { run: Some(_), .. }`, not `ClaimRefused`; the note is written; the run is `failed`).
  It does **not** claim "not re-queued": re-queueing lives in `crates/htui/src/run_worker.rs`, which
  `htui-orch` cannot see; that property is T3's `a_claim_time_refusal_is_not_requeued_on_postgres`
  (amended at fact-check). Conformance: D87's `a_capability_refusal_at_claim_fails_the_run_by_name`,
  added to `CASES` after T1's case with its `fn case` arm; pins 71 → 72 and the same count prose as
  T1. `status.rs` and `command.rs`: the byte rows from T1 stay and now pass through `htui_core`'s
  function.
- **Action**: D85; `RunFailure::MissingTags`'s `Display` and `EngineError::MissingTags`'s
  `#[error]` call `htui_core::model::missing_tags_failure` (amended at fact-check).
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; `cargo test -p htui
  --all-features -- --test-threads=1`.

### Task 3: Postgres end to end (D89)
- **Files**: `crates/htui/tests/runs_pg.rs`.
- **Tests**: `a_capability_refusal_blocks_and_unblock_reopens_on_postgres` and
  `a_claim_time_refusal_is_not_requeued_on_postgres`, as D89 describes (the second amended at
  fact-check).
- **Validate**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui --test runs_pg
  --all-features -- --test-threads=1`, then the workspace gate below after both Wave 2 lanes merge.

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named in the task. **The
first red tests of the milestone are T0's `missing_tags_failure_joins_in_the_given_order` and T1's
`a_capability_refusal_writes_no_run_and_unblock_reopens`**, one per Wave 1 lane. **No re-queue of
a claim-time refusal**: T3's `a_claim_time_refusal_is_not_requeued_on_postgres` (amended at
fact-check).

**Criterion 14, in full.** No `run` row, `item.status = 'blocked'`, note body exactly the missing
tags, `Unblock` reopens, and a subsequent run on a box with those tags succeeds: T1's conformance
case over `MemStore`; T3's end-to-end test over `PgStore` through the worker. **ANA-2 §4.10's
claim-time half**: T0's two store conformance cases over both `MemStore` and `PgStore`; T2's
`htui-orch` case over `MemStore`. **The seam**: T1's through-the-trait unit test, and T3 through
`BackendGraphs` on Postgres.

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| Store `CASES` | 74 | 76 (T0) | `crates/htui-core/src/store/conformance.rs` (`CASES` at `:43`); `crates/htui-core/tests/mem_store.rs:36-37`; `crates/htui-store/tests/pg_conformance.rs:19` |
| `READ_CASES` | 14 | 14 | `conformance.rs:291` |
| `htui-orch` `CASES` | 70 | 71 (T1), 72 (T2) | `crates/htui-orch/src/conformance.rs` (`CASES` at `:334`, pin test about `:5510`); `crates/htui-orch/tests/fake_conformance.rs:15-17` |
| `StoreRequest` / `StoreReply` | 68 / 39 | unchanged | `crates/htui/src/store_worker.rs` |
| `.sqlx` files | 264 | 267 (T0; exact figure recorded at close) | `crates/htui-store/.sqlx/` |
| Migrations | `0001`..`0007` | unchanged; the next is still `0008` | `crates/htui-store/migrations/` |
| `crates/htui/tests/snapshots` | 87 | 87 | — |
| `GraphSource` methods | 6 | 7 | `crates/htui-orch/src/graph.rs` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-33** — `Claim` loses `Copy`, and a caller somewhere copies it | **Confirmed at fact-check: one site**, `crates/htui-store/tests/pg_criteria.rs:694`/`:709` (E0382 without `Copy`), fixed in T0 by borrowing | T0's gate compiles every `htui-store` test target (`--no-run`) and builds `htui-orch` and `htui-agent` |
| **R-34** — Between the Wave 1 merges an engine reports a claim-time `MissingTags` as `ClaimRefused` and the worker re-queues a `failed` run | Low | `reclaim` drops a non-`queued` run (`run_worker.rs:1931-1933`); the window closes with T2; no release is cut mid-wave |
| **R-35** — A failed item refused at enqueue stays `failed`, not `blocked` (D82) | Low | The note is written; a run after the tags exist succeeds; recorded under disagreements |
| **R-36** — `required_tags` are unvalidated, so a case or spelling mismatch refuses a run the user thought was satisfied (OQ-25) | Medium | The refusal names the exact tag; validation is a follow-up |
| **R-37** — An existing `htui-orch` case starts an item whose `required_tags` the demo box lacks, and turns red | Low | Every `htui-orch` case starts `HTUI_FEAT_3` (`rust`, probed on the fixture box), `HTUI_ANA_2` (no tags) or a `mint_feat` item (empty tags); the full suite runs in T1's gate; claim to verify |
| **R-38** — An existing store conformance case claims a run of a tagged item on a box that lacks the tag; or a new case fails a snapshot-less seeded run through `claim_run`, which `ck_run_graph_snapshot` (`0003_orchestration.sql:47-48`, `NOT VALID` but checked on every `UPDATE`) refuses on Postgres | Low | T0's gate runs both stores' suites; T0's setup rule: every run a case fails through `claim_run` comes from `create_run` with a snapshot, never `RUN_2` (amended at fact-check) |
| **R-39** — The engine writes the note after the store transaction, so a crash in between leaves a `blocked` item with no note | Low | The same exposure rung 4 has today (`engine.rs:690-706`); `run.failure` carries the sentence at claim; the Backlog still shows the item `blocked` and `Unblock` works |
| **R-40** — Deviations the main thread must record: the note body (OQ-20), the store-side write at claim (OQ-21), check order (OQ-22), the `failed` item (OQ-23), the MOD-4 case's reworded doc | Medium | Listed under "Where the PRD, HANDOFF or tree disagree" |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
# the prepare check needs a scratch database migrated through 0007 (the compose `htui` database
# is empty; project memory):
docker exec htui-postgres psql -U postgres -c "CREATE DATABASE htui_prepare_check;"   # once
cd crates/htui-store && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check sqlx migrate run --source migrations && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features
ls crates/htui-store/.sqlx | wc -l                      # 267 (exact figure recorded at close)
cargo doc --workspace --no-deps --keep-going            # exactly the five baseline errors (HANDOFF.md:42-45)
git diff --stat 98e6d2f -- crates/htui-store/migrations crates/htui-store/cache_migrations \
  crates/htui/tests/snapshots crates/htui-core/src/fixtures.rs   # empty
```

`--test-threads=1` is not optional (the keyring fake is process-wide). Before believing a Postgres
failure, run `df -h /` (the dev Postgres crash-loops under disk pressure) and re-run the case alone.

**Live check on this box (optional; after Wave 2 merges).** Launch `htui` against the dev database.
On an `open` item with no tags, set a required tag this box lacks (for example `cuda`) through the
Backlog item editor if it exposes `required_tags`, else with one `UPDATE item SET required_tags`;
start a run (`R` in the Runs pane): the status line says `item …: missing tags: cuda`, the Backlog
shows the item `blocked` with the note, and no run row appears. `u` unblocks it. Declare `cuda` in
Settings > Boxes (`t`), start again: the run starts.

## Acceptance

- [ ] An item requiring a tag this box has neither probed nor declared is refused at enqueue with
      no `run` row, `item.status = 'blocked'` and a note whose body is `missing tags: …` naming
      exactly the missing tags in byte order, written with this box's `box_id` (criterion 14).
- [ ] `Unblock` returns that item to `open`, and after the box declares the tags a `StartRun` walks
      (criterion 14's second half, over `MemStore` and over `PgStore` through the worker).
- [ ] A `queued` run whose item's tags the box lost between enqueue and claim is failed inside the
      admission transaction with `run.failure = "missing tags: …"`, its item `blocked`, the same
      note, and no slot taken, on both stores; the check precedes `SlotFull` and `Overlaps`.
- [ ] A claim-time refusal is `EngineError::MissingTags`, never `ClaimRefused`, so nothing
      re-queues a failed run.
- [ ] The engine reads tags only through `GraphSource::missing_tags`; no store type is named in
      `htui-orch`'s engine.
- [ ] Store `CASES` 76 in all three places; `htui-orch` `CASES` 72 in both places; no migration;
      `.sqlx` regenerated and `prepare --check` clean; no snapshot or fixture moved.
- [ ] The workspace gate above is green.

## Where the PRD, HANDOFF or tree disagree

1. **ANA-2 §4.10 vs §12 criterion 14 on the note's body**: "names the box and the missing tags" vs
   "exactly the missing tags". OQ-20, D78.
2. **The PRD's constraint "Next migration is `0005`"** is obsolete: `0005`–`0007` have landed and
   `HANDOFF.md` says the next is `0008`. This milestone needs none.
3. **The PRD's Evidence cites `Backend::missing_tags` at `backend.rs:558` and `ready_items` at
   `:543`**; the tree has them at `:561` and `:546` (their docs start a few lines above). The PRD
   cites `GraphSource` as `graph.rs:57-101`; since MOD-9 the trait has six methods (`bound_skills`
   added) and ends later.
4. **ANA-2 §4.10 says the shipped ready query lacks the box join** (`ItemFilter.ready`); MOD-4
   milestone 1 has since shipped `ready_items(scope, box_id)` with the tag clause
   (`mem.rs:651-670`, `pg/read.rs:1863`). Informational; MOD-12's.
5. **ANA-2 §4.10 gives the predicate as `<@` and the message as an `EXCEPT`**; the shipped read
   uses `t <> ALL (…)` with `DISTINCT … COLLATE "C"`. Same set, and byte order is what both stores
   return; D80's claim query follows the shipped form.
6. **The MOD-4 case `unblock_opens_a_blocked_item_with_no_run` and the `htui-orch` `CASES`
   preamble call it "criterion 14's `Unblock` half"**, and `HANDOFF.md`'s MOD-7 entry says the
   same. After T1 criterion 14 has its own case; T1 rewords the two code comments (D86), and the
   HANDOFF text is the main thread's.
7. **`Claim`'s `#[must_use]` says "a refused claim wrote nothing"** (`overlap.rs:127`), and
   `EngineError::ClaimRefused`'s doc says the run stays `queued`; D80 and D84 make the first false
   for `MissingTags` (reworded in T0) and keep the second true by not using `ClaimRefused`.
8. **`MemStore::missing_tags`'s doc says the Backlog renders it**; nothing does. OQ-24; the doc is
   left as the intended consumer and the main thread decides on a follow-up.
9. **A `failed` item cannot become `blocked`** under the item law, so criterion 14's "sets
   `item.status = 'blocked'`" holds for an `open` item only. OQ-23, D82.
10. **The PRD's "Record corrections"** (the stale "no writer" comments at `engine.rs:4928` and
    `:5615`, the strip comment in `tests/settings.rs`, the HANDOFF text of MOD-7 and MOD-40) are
    not this milestone's: the first belongs to milestone 4's excerpt work, the second was done in
    milestone 2, the third is the main thread's at close.
11. **ANA-2 §4.10 cites `R-ORCH-10` at `docs/REQUIREMENTS.md:183-184`**; the requirement is at
    `:234-235` now. The ANA-2 citation is stale; recorded for the main thread (amended at
    fact-check).
12. **`GraphSource`'s doc says `app_setting` is "deliberately not a fifth method"**
    (`graph.rs:49`), but the trait has six methods since MOD-9 and gets a seventh here. T1 may
    reword it (amended at fact-check).

---

## Claims to verify

Every checkable fact this plan asserts, for the fact-check pass. Line numbers are at `98e6d2f`.

1. `graphify-out/` does not exist in the checkout.
2. `HEAD` is `98e6d2f`, and `crates/htui-store/.sqlx/` holds 264 files.
3. `crates/htui/tests/snapshots` holds 87 files.
4. The migrations are `0001`..`0007`; `HANDOFF.md` says the next is `0008` (`:29-31`).
5. `item.required_tags` is `TEXT[] NOT NULL DEFAULT '{}'` at `0001_init.sql:322`; `run.failure` is a plain nullable `TEXT` at `0001_init.sql:462` with no `CHECK` in any migration.
6. No `CHECK` on `run` in `0001`–`0007` forbids `status = 'failed'` with `started_at` and `executing_box_id` NULL (the shape `free_feat_3`'s cancellation of the queued `RUN_2` already writes, but for `failed`).
7. `step_graph_phase` carries no tag column, and `StepGraphPhase` (`crates/htui-core/src/model/kind.rs:177-212`) has no tags field.
8. `Item.required_tags: Vec<String>` is at `crates/htui-core/src/model/item.rs:139-140`.
9. `Status::can_move_to` (`item.rs:52-66`) allows `queued -> blocked` and `open -> blocked`, and not `failed -> blocked`; `blocked -> open` is allowed.
10. `RunStatus::can_move_to` (`crates/htui-core/src/model/run.rs:60-70`) allows `queued -> failed` and `running -> failed`.
11. `finish_run_item_mirror` (`crates/htui-core/src/store/traits.rs:1599-1608`) answers `None` for `(Failed, Queued)`.
12. `WriteStore::claim_run` is declared at `traits.rs:851-858` and documents the run looked up before the box.
13. `Claim` is at `crates/htui-core/src/model/overlap.rs:126-148`, derives `Debug, Clone, Copy, PartialEq, Eq`, has four variants (`Admitted`, `NotClaimable`, `SlotFull`, `Overlaps`), carries `#[must_use = "a refused claim wrote nothing; the caller must act on why"]`, and its only exhaustive `match` is its own `Display` (`:157-166`).
14. No code in the workspace relies on `Claim: Copy` (every use moves, borrows or compares it). **Falsified at fact-check:** `crates/htui-store/tests/pg_criteria.rs:694`/`:709` does; fixed in T0.
15. `Claim` is re-exported through `htui_core::model` (the `htui-orch` conformance imports `htui_core::model::Claim`).
16. `State::claim_run` is at `crates/htui-core/src/store/mem.rs:3550-3638`: run lookup, box lookup, `NotClaimable` at `:3565-3567`, then the slot count, then overlap, then the admitted writes, with the "a stale item status is not a refusal" branch at `:3629-3635`.
17. `State::box_capabilities` (`mem.rs:3346-3357`) is `probed_tags ∪ declared_tags` and empty for an unknown box.
18. `MemStore::missing_tags` (`mem.rs:678-696`) sorts by bytes and deduplicates; its doc (`:672-673`) says it is what the Backlog renders.
19. `MemStore::ready_items` is at `mem.rs:651-670` and filters by the tag subset.
20. `PgStore::claim_run` is at `crates/htui-store/src/pg/write.rs:2990-3132`, runs in one transaction, locks the run row and the box row `FOR UPDATE` (`:3001-3027`), returns `NotClaimable` at `:3029-3031` before reading the limit, and commits only on admission.
21. `PgStore::missing_tags` (`crates/htui-store/src/pg/read.rs:1919-1962`) uses `SELECT DISTINCT t COLLATE "C" … t <> ALL (b.probed_tags || b.declared_tags) ORDER BY 1`.
22. `PgStore::finish_run` (`pg/write.rs:4283-4389`) has an `UPDATE run SET status = $2, failure = COALESCE($3, failure), finished_at = COALESCE(finished_at, $4)` statement at about `:4331-4344`.
23. `Backend::missing_tags` is at `crates/htui-store/src/backend.rs:561-567` and `Backend::ready_items` at `:546-552`; `Backend::repo_paths` exists at `:532`.
24. `missing_tags` has no non-test caller: only `mem.rs:8966-8976` and `crates/htui-store/tests/pg_criteria.rs:3247-3280` call it (besides `Backend`'s dispatch).
25. `GraphSource` is declared at `crates/htui-orch/src/graph.rs:56` with the doc at `:33-55` naming inherent reads the engine cannot reach, and has six methods (`resolve_graph`, `phase_agents`, `prompt_template`, `agent`, `agent_boxes`, `bound_skills`).
26. `GraphSource` has exactly four implementations: `MemStore` (`crates/htui-orch/src/fake.rs:844`), `FakeGraphSource` (`fake.rs:963`), `TestSource` (`graph.rs:797`, test-only), `BackendGraphs` (`crates/htui/src/run_worker.rs:2366`).
27. `fake.rs:2117` is the doc of a unit test that calls `impl GraphSource for MemStore`'s methods through the trait ("delegates to the six inherent reads").
28. `Engine` is declared at `crates/htui-orch/src/engine.rs:459` with eight generic parameters `<'a, S, G, I, V, C, A, K>`, and `self.parts.graphs`, `self.parts.box_id` and `self.parts.user` exist.
29. `Engine::enqueue` is at `engine.rs:596-641`: `self.item`, `graph::resolve`, the rung-4 arm calling `refuse_rung_four`, then `create_run`; the comment about `open | failed -> queued` is at about `:621-623`.
30. `Engine::start_run` (`engine.rs:644-652`) is `enqueue` then `claim`.
31. `Engine::claim` (`engine.rs:666-683`) calls `claim_run` and returns `EngineError::ClaimRefused { run, claim }` for any non-admitted verdict.
32. `Engine::refuse_rung_four` (`engine.rs:690-706`, doc from about `:684`) does `transition(Open, Blocked)`, then `note` with `RunFailure::NoCandidateAgent`'s `Display`, and its doc says a `failed` item's compare-and-set answers `Ok(false)` with the note still written.
33. `Engine::note` (`engine.rs:2954-2974`) writes `box_id: Some(self.parts.box_id)` and `created_by: self.parts.user`.
34. `RunFailure` is at `crates/htui-orch/src/status.rs:39-100`, has no missing-tags variant, its `Display` is at about `:102-136`, and `run_failure_display_is_ana2s_bytes` is at `:405`.
35. `EngineError` is at `crates/htui-orch/src/command.rs:310-596`; `ClaimRefused` at about `:376-385` with a doc saying the run stays `queued`; `the_named_refusals_say_what_went_wrong` constructs `ClaimRefused` at `:1626`.
36. `unblock_enabled` (`command.rs:1135-1160`) answers `Reopen` for a `blocked` item with no active run.
37. The run worker re-queues only `ClaimRefused`: `reclaim` (`run_worker.rs:1919-1951`, arm at `:1944`) and `start_run` (`:2118-2175`, arm at `:2162`); other errors go to `ctx.refuse(err.to_string())`; `reclaim` returns early when the run is not `queued` (about `:1931-1933`).
38. `no_candidate_agent_blocks_the_item` is at `crates/htui-orch/src/conformance.rs:2574-2620` and `unblock_opens_a_blocked_item_with_no_run` at `:5200-5248`, whose doc calls it "criterion 14's `Unblock` half".
39. The `htui-orch` `CASES` list starts at `conformance.rs:334`, holds 70 names, and its preamble (about `:330-333`) mentions criterion 14's reopen.
40. The `htui-orch` count is pinned at 70 in `conformance.rs`'s `cases_are_unique_and_seventy` (about `:5510`) and in `crates/htui-orch/tests/fake_conformance.rs:15-17` (`cases_len_is_seventy`), and nowhere else.
41. `start_refused` (`conformance.rs:3538-3551`), `mint_feat` (`:3510`, empty `required_tags`), `free_feat_3` (`:777-782`), `item_of`, `notes_of` and `unblock` helpers exist; `Orchestrate::claim` exists (about `:133`) and `Orchestrate::store()` returns `&MemStore`.
42. `a_third_run_waits_for_a_slot_and_a_parked_run_still_blocks_overlap` (`conformance.rs:3661-3718`) leaves a run `queued` through an `Overlaps` refusal.
43. A case can read an `item_note`'s `box_id` through `MemStore`'s `ReadStore` (a notes read returning rows, not only bodies).
44. Every `htui-orch` conformance case starts only `HTUI_FEAT_3`, `HTUI_ANA_2` or a `mint_feat` item, none of whose `required_tags` the fixture box lacks.
45. Every existing store conformance case that calls `claim_run` claims a run whose item's `required_tags` the fixture box satisfies (or a chat run).
46. The fixture box (`crates/htui-core/src/fixtures.rs:458-480`) is `ids::BOX` with `probed_tags = [rust, msvc, cmake]`, `declared_tags = [gpu]`, `edit_version = 0`, `max_concurrent_items = 2`; `HTUI_FEAT_3` requires `[rust]`.
47. `WriteStore::update_item(id, expected_version, ItemPatch)` exists and `ItemPatch.required_tags: Option<Vec<String>>` exists (`item.rs:271-272`); `ItemPatch` implements `Default`.
48. `WriteStore::edit_box(id, expected: i32, BoxEdit)` exists on every store (`MemStore::edit_box` at `mem.rs:5284`) and `BoxEdit { declared_tags: Option<Vec<String>>, quirks: Option<String> }`.
49. The store `CASES` list is at `crates/htui-core/src/store/conformance.rs:43` with 74 names; `READ_CASES` at `:291` with 14; `claim_run_admits_one_and_refuses_the_second` at `:4158`; `edit_box_is_cas_on_edit_version` at `:5535`.
50. The store count is pinned at 74 in `crates/htui-core/tests/mem_store.rs:36-37` and `crates/htui-store/tests/pg_conformance.rs:19`, and nowhere else.
51. `crates/htui/tests/runs_pg.rs` has `Stack` (`:235`), `Stack::take_status` (`:297`), `Stack::notes` (`:376`), `Stack::start` (`:388`), `Stack::run_ids` (`:407`), `free_feat_3` (`:504`) and `unblock_follows_an_escalated_run_on_postgres` (`:940`); it uses runtime `sqlx` queries only, and `crates/htui` has no `.sqlx/`.
52. `StoreRequest` has 68 variants and `StoreReply` 39 (`HANDOFF.md` pins), and this plan changes neither.
53. `canonical_declared_tags` is at `crates/htui-core/src/model/box_.rs:184` and enforces `[a-z0-9_-]{1,64}`; nothing validates `item.required_tags`.
54. `HANDOFF.md` records exactly five `cargo doc` baseline errors (about `:42-45`).
55. T0 ∩ T1 = ∅ and T2 ∩ T3 = ∅ by the file lists above; T1 ∩ T2 is the four `htui-orch` files named.
56. T1 compiles against the base `htui-core` (it needs nothing T0 adds), and the base `htui-orch` and `htui-agent` compile against T0's `Claim`. **Falsified at fact-check:** D84 had T1's `#[error]` call T0's function, and `pg_criteria.rs` needs `Copy`; fixed by T1's literal sentence (T2 switches it, so `command.rs` joins T2) and T0's `pg_criteria.rs` edit.
57. `docs/ANA-2.md` §4.10 says the note "names the box and the missing tags" and the claim-time failure is `run.failure = "missing tags: a, b"`; §12 criterion 14 says the note body "is exactly the missing tags".
58. `docs/REQUIREMENTS.md:234-235` is `R-ORCH-10`.

---

## Verified claims

Filled by the fact-check pass.

| Claim | Verdict | Evidence |
|---|---|---|
| 1. no `graphify-out/` | verified | absent from the checkout |
| 2. `HEAD` `98e6d2f`, 264 `.sqlx` files | verified | `git log -1`; `ls crates/htui-store/.sqlx \| wc -l` = 264 |
| 3. 87 snapshots | verified | `ls crates/htui/tests/snapshots \| wc -l` = 87 |
| 4. migrations `0001`..`0007`, next `0008` | amended | true; the HANDOFF lines are `:31-34`, not `:29-31` |
| 5. `required_tags` `:322`, `run.failure` `:462` plain `TEXT` | verified | `0001_init.sql:322`, `:462`; no `CHECK` on `failure` |
| 6. no `CHECK` forbids a `failed` run with NULL `started_at`/`executing_box_id` | amended | no such check, but `ck_run_graph_snapshot` (`0003_orchestration.sql:47-48`, `NOT VALID`, checked on new rows and `UPDATE`s) refuses the `failed` update of a `kind = 'graph'` run with NULL `graph_snapshot`; `create_run` runs are fine; R-38 and T0's setup rule added |
| 7. no phase tags | verified | `kind.rs:177-212`; no tag column in `step_graph_phase` |
| 8. `Item.required_tags` | verified | `item.rs:139-140` |
| 9. item law edges | verified | `item.rs:52-66` |
| 10. run law edges | verified | `run.rs:60-70` |
| 11. `finish_run_item_mirror(Failed, Queued)` = `None` | verified | `traits.rs:1599-1608` |
| 12. `WriteStore::claim_run` doc and order | amended | `traits.rs:851-858`; its "a refusal writes nothing" doc at `:840-841` must be reworded (D80) |
| 13. `Claim` derives, variants, `must_use`, one exhaustive match | amended | `overlap.rs:126-148` as stated; `Display` is `:158-169`, its `match` `:160-167` |
| 14. no code relies on `Claim: Copy` | **falsified** | `pg_criteria.rs:694` `let refusal = if one.is_admitted() { two } else { one };` then `:709` `one.is_admitted()` → E0382 without `Copy` (probed on rustc 1.98.1); T0 gains the file and borrows |
| 15. `Claim` re-exported from `htui_core::model` | verified | the `htui-orch` conformance imports `htui_core::model::Claim` |
| 16. `State::claim_run` layout | amended | `mem.rs:3550-3638`; `NotClaimable` `:3566-3568`, admitted branch `:3628-3636`; its "writes nothing" doc `:3549` reworded (D80) |
| 17. `box_capabilities` | verified | `mem.rs:3346-3357` |
| 18. `MemStore::missing_tags` sort and doc | amended | `mem.rs:678-696`, doc `:672-673`; sort/dedup at `:692-693` (D76 corrected) |
| 19. `MemStore::ready_items` | verified | `mem.rs:651-670` |
| 20. `PgStore::claim_run` layout | amended | `pg/write.rs:2990-3132`; run `FOR UPDATE` `:3000-3016`, box `:3018-3028`, `NotClaimable` `:3030-3032`; "writes nothing" doc `:2988-2989` reworded (D80) |
| 21. `PgStore::missing_tags` SQL | amended | `pg/read.rs:1919-1962`; the SQL is `:1922-1926` (D76 corrected) |
| 22. `finish_run`'s `UPDATE run` | amended | the statement is `pg/write.rs:4328-4341` (Patterns corrected) |
| 23. `Backend::missing_tags`, `ready_items`, `repo_paths` | verified | `backend.rs:561-567`, `:546-552`, `:532` |
| 24. `missing_tags` has no non-test caller | verified | only `mem.rs:8966-8976` and `pg_criteria.rs:3247-3280`, besides `Backend`'s dispatch |
| 25. `GraphSource` at `graph.rs:56`, six methods | amended | the trait is `:57` (`:56` is the `#[allow]`); six methods as listed; its doc's "not a fifth method" (`:49`) is stale (disagreement 12) |
| 26. four `GraphSource` implementations | verified | `fake.rs:844`, `fake.rs:963`, `graph.rs:797`, `run_worker.rs:2366` |
| 27. `fake.rs:2117` through-the-trait test doc | verified | "delegates to the six inherent reads of the same names" |
| 28. `Engine` generics and `parts` fields | verified | `engine.rs:459`; `parts.graphs`, `parts.box_id`, `parts.user` |
| 29. `Engine::enqueue` | amended | `engine.rs:596-641`; the `open \| failed` comment is `:623-625` |
| 30. `start_run` | verified | `engine.rs:644-652` |
| 31. `Engine::claim` | amended | `engine.rs:666-683`; the comment is `:669-671` |
| 32. `refuse_rung_four` | amended | fn `:690-706`; the doc is `:685-689` (OQ-23, D82, Patterns corrected) |
| 33. `Engine::note` | verified | `engine.rs:2954-2974` |
| 34. `RunFailure` | verified | `status.rs:39-100`, `Display` about `:102-136`, byte test `:405` |
| 35. `EngineError`, `ClaimRefused`, sentence test | amended | `ClaimRefused` `command.rs:375-383`; test fn `:1614`, builds `ClaimRefused` at `:1626` and `:1637` |
| 36. `unblock_enabled` | verified | `command.rs:1135-1160` |
| 37. worker re-queues only `ClaimRefused` | amended | arms `:1944`, `:2162` as stated; `reclaim`'s early return is `:1933-1935` |
| 38. MOD-4 cases | amended | `:2574-2620` and `:5200-5248`; the "criterion 14's `Unblock` half" doc is `:5197-5199` |
| 39. `htui-orch` `CASES` at `:334`, 70 names | verified | as stated |
| 40. `htui-orch` count pins | amended | fn `:5510`, `70` at `:5517`, and `fake_conformance.rs:15-17`; the count also appears in prose at `conformance.rs:288-292` ("Seventy…", the pin-test name, "18 + 5 + 13 + 6 + 10 + 18") and `:507` ("seventy-arm match"), and each new case needs an arm in `fn case` (`:513`); T1 and T2 updated |
| 41. conformance helpers, `Orchestrate::claim`, `store()` | verified | as stated |
| 42. `:3661-3718` leaves a run `queued` through `Overlaps` | amended | that case never checks the `Overlaps`-refused run's status; `overlapping_touched_paths_serialise` (`:3557`, checks `:3571-3584`) does; D87 and Patterns cite it |
| 43. a note's `box_id` readable from `MemStore` | verified | the notes read returns rows |
| 44. `htui-orch` cases start only satisfied items | verified | `HTUI_FEAT_3` (`rust`), `HTUI_ANA_2`, `mint_feat` items |
| 45. store claim cases claim satisfied items | verified | as stated |
| 46. fixture box and `HTUI_FEAT_3` tags | verified | `fixtures.rs:458-480`; `HTUI_FEAT_3` requires `[rust]` |
| 47. `update_item`, `ItemPatch.required_tags`, `Default` | verified | as stated |
| 48. `edit_box` and `BoxEdit` | verified | `mem.rs:5284`; fields as stated |
| 49. store `CASES` 74, `READ_CASES` 14, case locations | verified | `conformance.rs:43`, `:291`, `:4158`, `:5535` |
| 50. store count pins | verified | `mem_store.rs:36-37`, `pg_conformance.rs:19` |
| 51. `runs_pg.rs` helpers, runtime queries | verified | as stated; note `PgStore::runs` still returns FEAT_3's seeded cancelled `RUN_2`, so D89 says `run_ids` is unchanged (`[ids::RUN_2]`), not empty |
| 52. `StoreRequest` 68 / `StoreReply` 39, unchanged | verified | HANDOFF pins; no variant added |
| 53. `canonical_declared_tags` rule; `required_tags` unvalidated | amended | the rule is `[a-z0-9_-]{1,64}` whose first character is a letter or digit (`is_declared_tag`, `box_.rs:167`); OQ-25 corrected |
| 54. five `cargo doc` baseline errors | verified | `HANDOFF.md` about `:42-45` |
| 55. T0 ∩ T1 = ∅, T2 ∩ T3 = ∅ | verified | by the file lists (T1 ∩ T2 now five files, with `command.rs`) |
| 56. T1 builds on base `htui-core`; base `htui-orch`/`htui-agent` build on T0's `Claim` | **falsified** | D84 had T1's `#[error]` call T0's `missing_tags_failure`, and `pg_criteria.rs` relied on `Copy`; fixed: T1 uses literals, T2 switches them (`command.rs` added to T2), T0 fixes `pg_criteria.rs` |
| 57. ANA-2 §4.10 and §12 criterion 14 wording | verified | as quoted; side note: §4.10 cites `R-ORCH-10` at `REQUIREMENTS.md:183-184`, which is stale (disagreement 11) |
| 58. `R-ORCH-10` at `REQUIREMENTS.md:234-235` | verified | as stated |
| **Task independence (wave 1 / wave 2)** | verified after fixes | Wave 1 (T0 ∥ T1): independent once T1 spells both sentences as literals and T0 owns the `pg_criteria.rs` `Copy` fix; file sets disjoint, `.sqlx` only in T0, count pins split. Wave 2 (T2 ∥ T3): independent, file sets disjoint |
