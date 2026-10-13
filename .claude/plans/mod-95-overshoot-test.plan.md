# Plan: MOD-95 - auto_queue overshoot test: the batch closes as drained

**Source**: `HANDOFF.md` MOD-95 (routed **plan**, 2026-10-13)
**Complexity**: Small - one test, one file, no product code

## Summary

`a_batch_overshoots_its_cap_by_at_most_one_attempt_pg` pins the batch open after the overshoot,
but the runtime closes it `drained`. **The behaviour is right; the test is stale.** The close is the
maintainer's MOD-12 review **L4** decision (2026-10-07, `docs/decisions/mod/mod-12.md:95`): a sweep
with a free slot and nothing admissible, including every entry held by the spend gate, closes the
batch `drained` once no run of its own is live, "so a later `Q` never starts spending; `P` does".
It landed in `03a60f46` (M3 T2, 21:41). CLEAN-9's pin `d0be442f` (23:35) was written on a branch that
did not contain `03a60f46`, so it encoded the pre-L4 world, where a cap-held entry left the batch
open. The two met only on merge.

Fix the test: assert the L4 close, and keep the post-close sweep as proof that a later entry
admits nothing and spends nothing until a resume.

## Decision

| Option | Verdict |
|---|---|
| A. Code: keep a cap-stalled batch open | **Rejected.** Reverses a recorded maintainer decision (L4), and breaks the Mem pin `a_batch_whose_every_entry_is_held_closes` (M3 (c)) and the module doc of `tests/auto_queue.rs:21-24`. |
| B. Test: pin `closed_reason = 'drained'`, then the later entry waits | **Chosen.** Matches L4, the Mem twin and `admit`'s doc (`runtime.rs:2169-2172`). |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Pg closed-row read | `tests/auto_queue.rs` `a_drained_batch_closes_drained_pg` | `SELECT closed_reason FROM queue_batch WHERE id = $1`, `assert_eq!(reason.as_deref(), Some("drained"))` |
| Open-batch read | `tests/auto_queue.rs` `open_batch` helper (Mem); `PgStore::open_batch_of` (`htui-store/src/pg/read.rs:2422`) | `store.open_batch_of(ids::BOX)` → `None` once closed |
| Assertion prose | `a_batch_whose_every_entry_is_held_closes` | "every entry held: it closed", messages say *why* |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-worker/tests/auto_queue.rs` | UPDATE | the overshoot test only: doc comment, the `:2202-2210` block, the final assertion's message |

## Tasks

### Task 1: re-pin the overshoot test to L4

- **Action**
  1. Doc comment (`:2075-2080`): replace "the next queue entry is not admitted" with the L4 outcome.
     With nothing admissible and its cut run ended, the batch closes `drained`, and an entry queued
     afterwards waits for a resume.
  2. Replace the open-batch block (`:2202-2210`) with:
     `closed_reason` reads `Some("drained")` ("L4: nothing admissible and no run of its own live,
     the overshot batch closed drained"); `db.store.open_batch_of(ids::BOX)` is `None`.
  3. Keep the `a2` queue + sweep + `pg_runs(a2).is_empty()` + unchanged `batch_spend`. Reword the
     message: no batch is open, so the later entry waits for `P` and nothing is spent.
- **TDD note**: this is a test-only correction of a red test. The "see it fail" step is the current
  failure (`left: Some("drained") right: None` at `:2210`, reproduced in this sandbox). Once
  re-pinned, check it is not vacuous: temporarily `return` before `drain(ctx, &batch,
  Drain::Stalled, ..)` and `Drain::Empty` in `admit` (`runtime.rs:2206/2263/2320`) and see the new
  assertion fail, then revert.
- **Validate**: `cargo test -p htui-worker --all-features --test auto_queue -- --test-threads=1`

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui-worker --all-features --no-fail-fast -- --test-threads=1
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Which drain arm closes (`Empty` vs `Stalled`) depends on whether `a1` is still ready after its failed run | Low | assert only the store-visible outcome (`drained`, no open batch), never the arm or its log line |
| Losing Pg coverage of "the cap declines admission in an open batch" | Low | under L4 that state only exists with a live batch run or no free slot; the cap-held close is pinned on Mem (M3 (c)) and the engine's cap refusal by `conformance.rs` `a_batch_at_its_cap_starts_no_attempt` |
| Other `_pg` suites red at the gate for unrelated reasons (load timeouts) | Medium | gate serially (`--test-threads=1`); compare to base `cae64f17` before calling a regression |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| The test fails at `:2210` with `Some("drained")` | **true** | sandbox run, `auto_queue` 0 passed / 1 failed, `left: Some("drained") right: None` |
| L4 (a stalled batch closes, including all entries cap-held) is a maintainer decision | **true** | `docs/decisions/mod/mod-12.md:95` "M1 review L4: a stalled batch closes (maintainer, 2026-10-07)" |
| The stall close landed in `03a60f46` | **true** | `git log -S "Drain::Stalled" -- runtime.rs` → only `03a60f46` |
| CLEAN-9's pin was written without the L4 code | **true** | `git merge-base --is-ancestor 03a60f46 d0be442f` → not an ancestor |
| `admit` closes when every ordered entry is stopped by the spend gate | **true** | `runtime.rs` `admit`: `admissible.is_empty()` → `drain(.., Drain::Stalled, ..)` |
| Mem already pins the cap-held close | **true** | `tests/auto_queue.rs` `a_batch_whose_every_entry_is_held_closes`: "every entry held: it closed" |
| `PgStore::open_batch_of` exists for the test to call | **true** | `crates/htui-store/src/pg/read.rs:2422` |
| `closed_reason` vocabulary is `paused`/`drained` | **true** | `0016_auto_queue.sql` `chk_queue_batch_closed_reason` |
| `admit` with no open batch returns before any read | **true** | `runtime.rs` `admit`: `Ok(None) => { queue_reads_ok(); return; }` |
| Task independence | n/a | single task, single file |

## Acceptance

- [ ] The overshoot test passes on Postgres and fails if the drain close is removed
- [ ] No product code changes
- [ ] Gate green; rust-reviewer run; MOD-95 closed out (write-up, DECISIONS, HANDOFF)
