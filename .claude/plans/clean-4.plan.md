# CLEAN-4: `LoopStop::NoProgressReview` is unreachable

Routed as **plan** (CLEAN default; C1–C4 ✗), ultracode not needed. The maintainer accepted on
2026-09-30. Sandbox run on `hr/CLEAN-4`. `R-ORCH-3`, risk R-9 (MOD-4 blueprint F-B, §11).

**Status: done 2026-09-30** (`docs/decisions/clean/clean-4.md`). Review M-1 narrowed D1's "two
latest review documents" to the reviews of the loop's last two turns (rows answered `Rejected`),
`d43ab17`.

## Goal

Make the review half of the review loop's no-progress predicate able to fire.
`gate::reviews_are_identical` (`crates/htui-orch/src/gate.rs:865`) reads through
`ReadStore::documents_of_kinds`, which returns **the latest version per kind**
(`crates/htui-core/src/store/traits.rs:103-116`). Because of that, `let [newest, previous, ..]` never
matches, and `LoopStop::NoProgressReview` (`gate.rs:814`) has been dead since MOD-4 M2. The fix reads
`documents()` heads (every version, without bodies), keeps this run's review-position steps, takes
the two highest versions, and fetches only those two bodies through `document(id)`.

**Behaviour change, deliberately.** CLEAN items are nominally behaviour-neutral. This one makes a
documented stop reason reachable, which is ANA-2 §4.4's intent all along. T1 pins every shipped case
first, so the one case whose stop changes is changed on purpose and in the open.

## Decisions

- **D1 — same predicate, fixed reader.** The semantics stay as documented on `no_progress`: take the
  two latest review documents produced by this run's steps at `review_position`, and compare
  `sha256(canonical(body))`. The hash half is still checked first, so a case where both halves hold
  still reports `no_progress_hash`. There is no new store method: `documents()` + `document()` are on
  `ReadStore` already and every backend implements them.
- **D2 — heads, then two bodies.** Filter heads by `kind == review output_kind` and
  `produced_by_step_id ∈ mine`, sort by `version` descending, and fetch two bodies. A head whose body
  read answers `None` (a row deleted between the two reads) answers `false`, never an error. That
  matches the function's existing "cannot tell → no stop" shape.
- **D3 — the one flipped case keeps its intent.**
  `gix_isolator::two_committing_implement_attempts_never_look_identical_in_worktree_mode` asserts that
  "the hash predicate did not fire, so the budget's third attempt ran". Its two reviews are the fake's
  default `"scripted output"`, so after the fix the *review* half would fire and park at attempt 2.
  The test scripts two distinct review bodies, which is how the conformance "exhausted" cases already
  do it. It then keeps pinning what it names: hash-predicate negativity over real git.
- **D4 — new conformance case `identical_reviews_stop_the_loop`**, the twin of
  `identical_after_hash_stops_the_loop`: `retry_limit = 3` on `implement`, **no repo in scope** (the
  hash half answers false on an empty scope), and review attempts 1 and 2 scripted with the same
  body. It expects `ReviewLoopExhausted(2)`, the note `stop reason \`no_progress_review\``, and no
  attempt 3. It runs on the fake (`tests/fake_conformance.rs`). CASES goes from 73 to 74 in all four
  pins.
- **D5 — canonicalisation is pinned at unit level.** The new `gate.rs` test next to
  `no_progress_compares_the_two_winners_not_index_zero` uses two review bodies that differ only in
  `\r\n` vs `\n`, which gives `Some(NoProgressReview)`. Two bodies that differ in content give `None`.
  A same-kind review document written by a step **outside** `review_position` does not count.

## Tasks (serial — shared files, no fan-out)

### T1 — Pin the stop reason of every shipped loop case (tests only, green on the baseline)

- `crates/htui-orch/src/engine.rs` `a_review_rejection_loops_then_escalates` (~`:7710`): add
  `stop reason \`exhausted\`` to the note assertion. Today the assertion checks only the
  reason-agnostic prefix `review loop exhausted after 2 attempts`.
- `crates/htui-orch/tests/gix_isolator.rs`
  `two_committing_implement_attempts_never_look_identical_in_worktree_mode`: script
  `ScriptedStep::review("request-changes", "first"/"second")` for review attempts 1 and 2 (D3), and
  extend the doc comment with the reason.
- Leave the already-pinned cases untouched. Record them in the commit body: conformance
  `review_rejection_loops_then_escalates` / `a_never_gate_rejection_escalates_when_the_budget_is_out`
  (`exhausted`), `identical_after_hash_stops_the_loop` and `gix_isolator.rs:1742` (`no_progress_hash`),
  engine `:7829` (`no_progress_hash`), and `tests/review_loop.rs:238` (`LoopStop::Exhausted`).
- **Validate:** `cargo test -p htui-orch --all-features` green before any `gate.rs` change.
- Commit: `test(clean-4): pin the stop reason each shipped review-loop case reaches`.

### T2 — Red: the tests that reach `NoProgressReview`

- `gate.rs` unit tests (D5): three cases in one test or three tests, mirroring
  `no_progress_compares_the_two_winners_not_index_zero`'s fixture (`MemStore::demo`, `run_3`, `step`).
- `conformance.rs`: `identical_reviews_stop_the_loop` (D4). Add it to `CASES` after
  `identical_after_hash_stops_the_loop` and to the dispatch `match`. Bump the count to 74 at
  `conformance.rs:310`, `:548`, `:5891`, and `tests/fake_conformance.rs:17`. Add a "One for CLEAN-4"
  paragraph to the CASES doc.
- **Validate:** the new tests fail against the unfixed `reviews_are_identical` (they get `None` or
  `Exhausted`). Commit: `test(clean-4): …` (red).

### T3 — Fix `reviews_are_identical`

- Rewrite the reader per D1/D2. Update its doc comment and `no_progress`'s doc (`gate.rs:~790-799`)
  so they name `documents()` heads and no longer imply the old reader.
- **Validate:** `cargo test -p htui-orch --all-features` green, including T1 and T2.
  Commit: `fix(clean-4): the review half of the no-progress predicate reads every version`.

### T4 — Gates and close-out

- `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace --all-features -- --test-threads=1` (memory: scheduling-dependent suite),
  including `htui-store --test pg_criteria` against the sandbox Postgres.
- Reviewer gate: `rust-reviewer` (`.claude/workflow-config.json`).
- Bookkeeping per `lifecycle.md` P2: `docs/decisions/clean/clean-4.md`, DECISIONS index line,
  HANDOFF line removed + summary + status line. Add a one-line "resolved by CLEAN-4" pointer under
  `docs/decisions/mod/mod-4.md` "Carried" R-9. Run the validator.

## Files

| File | Task | Action |
|---|---|---|
| `crates/htui-orch/src/engine.rs` | T1 | UPDATE (one assertion) |
| `crates/htui-orch/tests/gix_isolator.rs` | T1 | UPDATE (two scripts + doc) |
| `crates/htui-orch/src/gate.rs` | T2, T3 | UPDATE (unit tests; reader + docs) |
| `crates/htui-orch/src/conformance.rs` | T2 | UPDATE (new case, CASES, count docs, `:5891`) |
| `crates/htui-orch/tests/fake_conformance.rs` | T2 | UPDATE (73 → 74) |
| `docs/decisions/clean/clean-4.md`, `DECISIONS.md`, `HANDOFF.md`, `docs/decisions/mod/mod-4.md` | T4 | CREATE / UPDATE |

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Predicate unit test | `gate.rs:1501` `no_progress_compares_the_two_winners_not_index_zero` | `MemStore::demo`, `GateContext` by hand, `no_progress(&ctx, &steps, …)` asserted with a reason message |
| Conformance twin | `conformance.rs:1660` `identical_after_hash_stops_the_loop` | `repoint` retry_limit 3, drive with `approve`/`answer`, assert failure, note text, no attempt 3 |
| Distinct review bodies | `conformance.rs:1526-1527` | `orch.script("review", n, ScriptedStep::review(verdict, body))` |
| Store reads | `traits.rs:89,103` | heads for lists, `document(id)` for bodies |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A shipped case other than the gix one silently changes reason | Low | Probe run (below) + T1 pins the reason in every case that stated only the prefix |
| Crates outside htui-orch reach the loop with identical fake reviews | Low | Probe ran `pg_criteria` green. The T4 workspace gate covers the rest |
| Review fan-out: several review docs per attempt | Low | Same "two highest versions of this position" semantics as before the fix. A review phase with fan-out is not shipped. Noted in the write-up, not solved here |

## Verified claims

Fact-checked against the tree on 2026-09-30. The probe applied the D1/D2 reader as a throwaway patch,
ran the suites, and reverted it (`git checkout crates/htui-orch/src/gate.rs`).

| Claim | Verdict | Evidence |
|---|---|---|
| `reviews_are_identical` reads via `documents_of_kinds` | ✓ | `gate.rs:865-903` |
| `documents_of_kinds` is latest-per-kind | ✓ | `traits.rs:103` doc; `mem.rs:1148-1167` `latest_document` |
| `documents()` returns every version, without bodies | ✓ | `pg/read.rs:245-264` `ORDER BY kind, version`; `mem.rs:5636` `document_heads` "ascending by version" |
| `document(id)` returns the body, on `ReadStore` | ✓ | `traits.rs:97-103` |
| `DocumentHead` carries `id`, `kind`, `version`, `produced_by_step_id` | ✓ | the probe patch compiled |
| Only `Display` of `NoProgressReview` is tested | ✓ | `grep LoopStop::` shows `gate.rs:1362` only |
| Hash half is checked before the review half | ✓ | `gate.rs:810-815` |
| With the fix, exactly one htui-orch test changes outcome | ✓ | probe: `gix_isolator::two_committing_…_worktree_mode` panics at `parked()` (`:282`); all others green (459 unit, 36 gix minus 1, review_loop 4, infer 12, fake_conformance 3) |
| With the fix, `pg_criteria` stays green | ✓ | probe: 47/47 |
| That gix test passes on the baseline | ✓ | reverted, re-ran: 1 passed |
| Conformance "exhausted" cases already use distinct review bodies | ✓ | `conformance.rs:1526-1527` ("first"/"second"), `:2033-2045` |
| engine `a_review_rejection_loops_then_escalates` pins only the prefix, with distinct bodies | ✓ | `engine.rs:~7567-7571` scripts; `:7710` assertion |
| The fake's default output is one fixed string | ✓ | `fake.rs:1326` `done_with_output("scripted output")` |
| CASES count 73 is pinned in 4 places | ✓ | `conformance.rs:310`, `:548`, `:5891`; `tests/fake_conformance.rs:17` |
| Tasks independent? | ✗ — serial | T2/T3 share `gate.rs`, and T1 must be green before T3. No fan-out |
