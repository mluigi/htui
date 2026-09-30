# CLEAN-4 - `LoopStop::NoProgressReview` is unreachable (done, 2026-09-30)

**Requirements:** `R-ORCH-3`.
**Origin:** MOD-4, risk R-9 (`.claude/plans/mod-4-orch-fanout.blueprint.md` F-B and §11, carried
through milestones 5 and 6, `docs/decisions/mod/mod-4.md` "Carried").

**What was done.** The review loop's no-progress predicate (ANA-2 §4.4, MOD-4 plan D11) has two
halves. Only the `after_hash` half could fire. `gate::reviews_are_identical` read through
`ReadStore::documents_of_kinds`, which returns the latest version per kind. It therefore never held
two reviews to compare, and `LoopStop::NoProgressReview` had been unreachable since MOD-4
milestone 2. The review half now works. Plan `.claude/plans/clean-4.plan.md` (D1-D5, fact-checked
with a throwaway probe of the fix), blueprint `.claude/plans/clean-4.blueprint.md`.

- **Pins first (`fec35ac`).** Before `gate.rs` changed, every shipped loop case stated the stop
  reason it reaches. Engine `a_review_rejection_loops_then_escalates` now asserts `exhausted`;
  before, it asserted only the reason-agnostic `review loop exhausted after 2 attempts` prefix.
  gix `two_committing_implement_attempts_never_look_identical_in_worktree_mode` scripts two distinct
  review bodies (plan D3). The fake's default output is one fixed string, so with the fix its two
  default reviews would stop the loop at attempt 2 on `no_progress_review`, and the test would stop
  proving what it names: the hash half stays false over real git, so the budget's third attempt
  runs. The probe found this case, and it was the only htui-orch case that changed.
- **Red, then the fix (`c57a6fa`, `448a811`).** Two `gate.rs` unit tests: line endings are
  canonicalised away, and only this loop's reviews count, with a control leg so the negatives cannot
  pass on an empty read. One conformance case, `identical_reviews_stop_the_loop`, twins
  `identical_after_hash_stops_the_loop` with no repo in scope. `CASES` goes from 73 to 74 at all
  five pin sites. The reader moved to `documents()` heads plus two `document(id)` body reads.
- **What a loop turn is (review M-1, `d43ab17`).** The first fix compared the two latest review
  documents at the review position. That can pair a review answered `Retried` with its re-run, a
  crash-interrupted review with its reset re-run, or two versions a promoted review's chat wrote. The
  loop then stops when it made progress, or misses a real repeat. A turn is now a review row answered
  `Rejected`. Both entry points stamp that before the loop reads the steps: `reject_step` in
  `apply`'s `never` × rejected arm, and `Engine::answer_gate` for a human. The recovery sweep's
  `settle_failed` re-enters only on a row already stamped. The two rejected rows with the highest
  attempts are compared, each through the newest `output_kind` document it produced.
  `transition_step` leaves `gate_outcome` alone on both backends, so a loop-retired row
  (`failed -> cancelled`) is still a turn. The code assumes one turn per attempt: `review` never
  fans out (plan D64), and a candidate's rejection writes no `gate_outcome`.
- **The hash half is asked first**, unchanged. A loop where both halves hold still reports
  `no_progress_hash`. Three shipped cases depend on that order: conformance
  `identical_after_hash_stops_the_loop`, engine `two_identical_after_hashes_stop_the_loop_early`, and
  gix `:1742`.

**Behaviour change, deliberately.** CLEAN items are nominally behaviour-neutral. This one makes a
documented stop reason reachable. When a loop's two rejected reviews repeat each other, it now
escalates at that attempt with `stop reason \`no_progress_review\``. Before, it went on until the
budget or the hash half stopped it. Outside htui-orch, one shipped walk changes reason:
`crates/htui/tests/runs_pg.rs` `unblock_follows_an_escalated_run_on_postgres`. Its `OutputAuthor`
writes one body for every step, so it stops at the same attempt as before but on
`no_progress_review` instead of `exhausted`. The test now pins that reason. The pin was
negative-probed against the sandbox Postgres: flipped to `exhausted`, it fails.

**Review.** `rust-reviewer` approved with fixes. It raised one MEDIUM (M-1 above) and three LOWs:
- `runs_pg`'s reason flip: pinned.
- The kind filter was untested: a leg added.
- The missing-body branch is untested: accepted. Documents are append-only, so that branch is
  reached only by an item-delete race.

A re-review of `d43ab17` approved with four LOWs, all applied in `357e280`:
- The doc named a nonexistent `reject`.
- A 180-column doc line.
- The newest-version-per-turn choice was unpinned; a chat-rewrite leg now pins it.
- The one-turn-per-attempt assumption is now written down.

**Verification.** In the `hr/CLEAN-4` sandbox:
- `cargo fmt --all --check` passes.
- `cargo clippy -p htui-orch -p htui --all-targets --all-features -- -D warnings` is clean.
- `cargo test --workspace --all-features -- --test-threads=1`: 2962 passed, 0 failed, including
  `runs_pg` and `pg_criteria` against the sandbox Postgres.
- `cargo test -p htui-orch --all-features` after `357e280`: 519 passed, 0 failed.

Workspace-wide clippy is **red on main, and not because of this change**: four
`clippy::disallowed_methods` (raw `tokio::spawn`) in `crates/htui-agent/tests/loopback.rs:67,85,802,1013`.
That file came in with the MOD-22 merge and CLEAN-4 does not touch it. It was reported to the
maintainer and not fixed here.

**Commits.** `6bacbc3` plan, `b7fde67` blueprint, `fec35ac` pins, `c57a6fa` red tests, `448a811`
fix, `d43ab17` review M-1/L-1/L-2, `357e280` re-review LOWs, plus this close-out.
