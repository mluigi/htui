# CLEAN-11 - CLEAN-9 deferred review residuals (done, 2026-10-14)

**Requirements:** `R-AGT-7`.
**Origin:** CLEAN-9 (`docs/decisions/clean/clean-9.md`, review residuals).

**What was done.** Run in a sandbox on `hr/CLEAN-11`, routed as a plan (only C3 fired). Plan
`.claude/plans/clean-11-clean-9-residuals.plan.md` (D1, D2). No migration, no `.sqlx` entry.

- **Once-per-batch log levels pinned (`7a9f9bba`).** `htui-worker` takes `tracing-subscriber` as a
  dev-dependency. A `LogBuffer` capture (the shape `agent_worker.rs` uses) asserts that
  `Shared::note_batch_stop` says a batch's first stop at `INFO` and `note_bad_cap` its first malformed cap at
  `WARN`, and that every later one is `DEBUG`. Seen red with the two level macros of `note_batch_stop` swapped.
- **Project-settings decode warn deduped (`d2092889`, review R1 `2c6ec4e4`).** `graph::project_settings` warns once
  per project and edit, then at `debug`. A process-wide `OnceLock<Mutex<HashMap<ProjectId, i64>>>` remembers the last
  `updated_at` (micros) noted for each project, so it holds one entry per project (review M1: the first draft kept a
  set of every edit and grew without bound). `first_decode_failure` takes the map as a parameter so its unit test uses
  its own; the log test mints a fresh `ProjectId`, because the map is shared by the whole test binary.
- **`null` project settings (D1, `602b9f16`).** `ProjectCaps::from_settings` refuses `null` as a `CapError::document`,
  like every other non-object, so reader and writers agree (the Pg and Mem `set_*`/`clear_*` already refuse a
  non-object). Before it a `null` blob read as "no caps". No production path or fixture stores one: `create_project`
  writes `{}` and the Pg column defaults to `'{}'`, so only a hand-written `null` row is affected, and it is now refused
  at `graph::resolve`, held as `Hold::BadCap` in the queue overview and skipped by the worker's queue until fixed.
  The doc comment that called the looser read intentional is rewritten.

**Review.** `rust-reviewer`: no CRITICAL or HIGH, 1 MEDIUM (the growing once-set, applied), 5 LOW (the unreachable
`Null` arm is kept with a comment; the `LogBuffer` copy per crate, the callsite-interest note and the formatted-output
needle are accepted or applied as noted in the code).
**Gate.** `cargo fmt --check`, clippy with and without `--all-features`, `cargo test --workspace --all-features
--no-fail-fast -- --test-threads=1` (6220 passed, 0 failed, no stack overflow), validator. The host's Postgres gate on
the merged tree is owed.
**Process notes.** The Task 2 and Task 3 tests were seen red (Task 2 by not compiling before `first_decode_failure`
existed, Task 3 by the changed assertion before the guard moved).
