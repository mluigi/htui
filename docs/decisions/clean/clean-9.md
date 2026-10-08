# CLEAN-9 - MOD-12 M2 review residuals (done, 2026-10-08)

**Requirements:** `R-AGT-7`, `R-TUI-8`.
**Origin:** MOD-12 M2 (`.claude/plans/mod-12-m2-spend-guard.plan.md`, review D11/M3).

**What was done.** Run in a sandbox on `hr/CLEAN-9`, routed as a plan. Plan
`.claude/plans/clean-9-mod-12-m2-residuals.plan.md` (D1-D4). No migration; one `.sqlx` entry replaced.

- **Recorder row wording (D1).** `RunCap` carries `basis: CapBasis { Run, Batch(BatchId) }`. The `cap_exceeded`
  row reads "session allowance reached: an estimated $X (n micros) spent against an allowance of $Y (m micros), the
  remainder of project.settings.per_token_cap_run" (a batch basis names the batch and `per_token_cap_batch`).
  `Engine::open_recorder` derives the basis from `Allowance.batch`; `run_chat` passes `Run`. `5d27f934`.
- **One malformed key no longer uncaps the engine (D2).** `graph::resolve` reads the caps first through
  `ProjectCaps::from_settings` and refuses a bad cap key with the new `ResolveError::ProjectCaps`, so no run row is
  created. The lenient decode of the other keys stays and warns. `Engine::project_settings` reads no cap. A
  snapshotted run whose live blob has since gone bad keeps walking (`resume_window`), with a `warn` log
  instead of an item note. `2e8a2849`, `1129d089`.
- **Non-object `project.settings` (D3).** Pg `clear_setting` and `clear_queue_setting` carry
  `jsonb_typeof(settings) = 'object'` and answer `Stale` first, then `Constraint(project_settings_not_an_object)`,
  the order Mem uses. The refusal sentence is verb-neutral (`849df2d9`). `ProjectCaps::from_settings` returns a
  `CapError` for a non-object, non-null blob (`CapError::document`, `3eccc8ae`). Side effect, intended: the worker
  skips such an entry, a chat start is refused, and an enqueue is refused. `56f2d8e3`.
- **Test hardening (D4).** `a_batch_overshoots_its_cap_by_at_most_one_attempt_pg` asserts the batch is still
  open before the second sweep (`d0be442f`). The log-level pin needs a tracing capture dev-dependency and is
  CLEAN-11.

**Review.** `rust-reviewer`: no HIGH, 2 MED, 6 LOW, all applied except the deferred ones in CLEAN-11.
**Gate.** fmt, clippy with and without `--all-features`, `cargo test --workspace --all-features
--no-fail-fast -- --test-threads=1`.
**Process notes.** The Pg clear test and the T2 tests were written after the code, so they were not seen failing; the
reviewer reasoned the Pg test red without the guard (`[] - 'text'` succeeds, a scalar raises).
