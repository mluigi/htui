# MOD-73 - Hand-written document versions as step inputs (done, 2026-10-03)

**Requirements:** `R-ENT-12` (documents versioned per item and kind, produced by steps or written by
hand), `R-ORCH-2` (a gate answer may edit the artifact).
**Origin:** MOD-13 milestone 5 (`docs/decisions/mod/mod-13.md` "Carried"): Docs `v` wrote a
hand-written version that every view showed but the next phase never read.
**Artifacts:**
- plan [`.claude/plans/mod-73-hand-written-inputs.plan.md`](../../../.claude/plans/mod-73-hand-written-inputs.plan.md): D1-D6, with its verified-claims table;
- blueprint `.claude/plans/mod-73-hand-written-inputs.blueprint.md`: deviations B-1-B-8, hazards H-1-H-7, decisions E1-E9.

Decision numbers are local to MOD-73 (the MOD-31 convention).

Routed as **plan** (1 criterion fired: C3, the two options the item text named). Run in a TOOL-7
sandbox (`hr/MOD-73`). T1 ran first; T2, T3 and T4 had disjoint file sets and ran in parallel.

**Decisions (maintainer, 2026-10-03):**
- route accepted, no ultracode;
- plan confirmed as written and fact-checked: "a newer hand-written version wins", not "`accept
  artifact` adopts it" (D1);
- review: M1 fixed in this item rather than filed; L1-L3 and N1-N2 applied.

**Commits:**
- plan and blueprint: `915738e5`, `f89aba3a`;
- T1 rule, `MemStore`: `ab77b95d` (red), `1df035f8`;
- T2 Postgres, mirror, `.sqlx`: `98bcfb49` (red), `a85c079a`;
- T3 engine case: `b2046c33`;
- T4 ANA-2 §4.2: `a864e239`;
- review fixes: `e33366df` (red), `4f7d131b` (M1); `8ad7a9ff` (L1); `c39f3d9b` (L2, L3);
  `6687546f` (N1, N2).

---

## What was built

ANA-2 §4.2's gate-answer table already said "edits the artifact" is `approved` plus a new document
version. The resolver that feeds a phase its inputs ranked this run's output, then another run's,
then a hand-written document, **whatever the version**, so the edit never reached the next phase
while any step had produced that kind.

`ReadStore::resolve_inputs` now answers, per requested kind, **the higher version of two picks**:

- the **step-produced** pick, as before: rows with a `produced_by_step_id`, fan-out losers
  excluded, this run's output first, then another run's, then a row whose step is not held, then
  the highest version;
- the **hand-written** pick: the highest version with no producing step.

`document.version` is unique per `(item, kind)` and allocated in write order, so "higher" is
"written later". A gate edit, written after the step's output, is what the next step reads. An
older hand-written version still loses to this run's fresh output, and another run's output still
never overrides this run's.

- `MemStore` (`htui-core/src/store/mem.rs` `State::resolve_input`): two iterator picks, the larger
  version wins.
- Postgres (`htui-store/src/pg/read.rs`) and the SQLite mirror (`cache/read.rs`): the existing
  `ROW_NUMBER()` is partitioned by `(kind, produced_by_step_id IS NULL)` as `rank_in_arm`, and an
  outer `ROW_NUMBER()` per kind by `version DESC` picks the newer arm. It is one statement both
  engines run (blueprint H-14 holds). One `.sqlx` entry was replaced (`41886abe…` → `3a0fbf3c…`).
- Docs `v` (`htui/src/hand_written.rs` `v_base`, review M1): the form is prefilled from
  `resolve_inputs` for that kind, seated on the item's most recent run (a fresh `RunId` when it
  has none), not from `documents_of_kinds`. So an edit made at a fan-out gate starts from the
  selected output, never a loser's (`R-ORCH-7`). The Docs pane's lost-answer check reads the same
  request, and its rule still holds: a landed hand-written write is never above the answer.
- ANA-2 §4.2 carries the amendment: the sketch, the hand arm, the D3 column choice, and the
  gate-answer row.

## Decisions as built (plan D1-D6)

- **D1, a read rule, not `accept artifact`.** Adopting the edit would rewrite append-only rows or
  copy them, and would only cover an edit made at a gate. `accept artifact` stays guarded on a
  document produced by the promoted step itself (§4.8).
- **D2, the two arms** as above. With no output of this run, the item's history is read
  newest-first across hand-written documents and other runs, so a re-run reads an edit made at the
  previous run's gate.
- **D3, "hand-written" is `produced_by_step_id IS NULL`, not "the step row is missing".** They
  coincide on Postgres (`ON DELETE SET NULL`) and `MemStore`. On the mirror mid-refresh, a fan-out
  loser's document can briefly have no step row. Keyed on the column, it stays in the
  step-produced arm at rank 2 and cannot win on version. Pinned by
  `a_document_whose_step_has_not_arrived_is_never_hand_written` (`htui-store/tests/cache.rs`,
  review L1): flipping the partition to the join fails each of its two assertions on its own.
- **D4, one statement on both engines.** Probed on the sandbox Postgres and SQLite 3.50.4 before
  and after the blueprint.
- **D5, no migration.** `0003`'s `input_kinds` comment does not mention hand-written documents.
- **D6, the review loop inherits it.** A hand-written `review` newer than the rejecting one is what
  the re-run implement reads. `gate.rs::no_progress` reads reviews by step and is unaffected.

**Deviations (blueprint, review):**
- **B-1.** `htui-orch` `CASES` is pinned twice (`conformance.rs`, `tests/fake_conformance.rs`).
  Both moved 93 → 94 with the new case `a_gate_edit_is_what_the_next_phase_reads`. It asserts the
  next phase's recorded prompt carries the edit's `documents:prd` section, and `resolve_inputs` at
  the run's seat agrees. On the pre-MOD-73 rule it fails with `version="1"`.
- **B-5 / review L3.** ANA-2's retention paragraph now says the sweep's loser skip is load-bearing
  twice over. A future `run_step` sweep must also tombstone mirrored steps, because the refresher
  only upserts them; otherwise Postgres would move a swept step's document to the hand arm while
  the mirror kept it in the step-produced arm.
- **B-6.** `docs/ANA-5.md:1283` and `docs/ANA-10.md:1114` still quote the old `ORDER BY`. They are
  historical citations in concluded analyses; ANA-2 §4.2 is the authority.
- **Review L2.** The hand arm is any stepless document, not only a human edit: close-out's
  `summary` (`htui-orch/src/closeout.rs`) is one. The trait doc and ANA-2 say so. No seeded phase
  reads `summary`.

## Gate

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -D warnings` | clean |
| `SQLX_OFFLINE=true cargo check --workspace --all-features --all-targets` | clean |
| `cargo sqlx prepare --check` (`crates/htui-store`, migrated scratch DB) | exit 0 |
| `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` | 109 suites, 4210 passed, 0 failed, 30 ignored; no SIGABRT (on `4f7d131b`) |
| `validate-workflow-docs.sh` | 0 errors, 0 warnings |

Before the review fixes (`b2046c33`) the same test gate was 4208 passed, 0 failed.

**Pins:** store conformance `CASES` 135 and `READ_CASES` 15 (unmoved), `htui-orch` `CASES` 94,
322 `.sqlx` files (one replaced), no snapshot moved, no migration. `CASES` pins are bumped by other
open branches too; on merge, the value is the sum of the bumps.

## Carried

Nothing minted.
