# TOOL-15 - One integration-test binary per crate (done, 2026-11-13)

From the 2026-11-13 hr build-cache trial. Each of the 136 files under `crates/*/tests/` was its own test target,
so cargo linked 136 executables of 130-340 MB against the same dependency tree. Plan:
`.claude/plans/tool-15-one-test-binary-per-crate.plan.md` (decisions D1-D8; routed as plan by maintainer
override). Tooling: `.claude/plans/tool-15-one-test-binary-per-crate.tools/` (`names.sh`, `measure.sh`,
`after.sh`).

## Decision

- **Layout (D1).** Every crate has `tests/it/main.rs`, which declares one `mod <old-name>;` per old file; the files
  moved with `git mv` to `tests/it/<old-name>.rs`. No `Cargo.toml` changes. Run one file as
  `cargo test -p <crate> --all-features --test it <name>::` (a libtest substring filter, so `settings::` also runs
  `box_settings::`).
- **Two top-level exemptions (D2).** `htui-agent/tests/extensibility.rs` (its path is hard-coded in the `htui-dev`
  mod) and `htui/tests/panic_hook.rs` (it installs a process-wide panic hook that would see every parallel test's
  panics; added in the fix round after the Opus review).
- **Process-wide state (D3): per-test guards, no `cargo-nextest`.** The keyring fake already takes a process-wide
  lock; the workspace forbids `unsafe`, so no test mutates the environment. Five self re-exec sites now build the
  child's `--exact` name from `module_path!()`, and every parent asserts that the child ran exactly one test
  (`infisical.rs` gained that assertion). nextest stays the fallback if an order dependency ever needs a process
  per test.
- **Snapshots (D4).** 270 `.snap` files renamed to `tests/it/snapshots/it__<file>__<test>.snap`, contents untouched.
- **Feature canary (D5).** htui (`testkit`), htui-core and htui-agent (`test-support`) carry a root test that panics
  when the feature is off, so a featureless run of the merged binary fails instead of passing with 0 tests.
  Intended consequence: featureless `cargo test -p htui` is now red.
- **One binary per crate, not per workspace (D6).** `CARGO_BIN_EXE_htui` is set only for `htui`'s own tests.

## Built

`bfdadcfd`/`0c4bce47` plan and base measurement; T1 `30036547` (secrets), `1c8ecff7` (worker), `1c77d6f8` (orch),
`6f7ab01c` (mcp); T2 `dc339a4f` (core); T3 `7088a292` (store); T4 `c4d5080a` (agent); T5 `bd979072` (htui);
`3dbaaffe` README, `docs/htui-secrets.md`, `run_loop.sh`, `AGENTS.md`; `985dde93` review and gate fix round;
`fe802740` `after.sh`. Branch `tool-15`, 431 files changed (almost all renames).

## Measured (host, `CARGO_BUILD_JOBS=4`, fresh `CARGO_TARGET_DIR`, same recipe both sides)

| | base `ae910789` | after `fe802740` | D8 bound |
|---|---|---|---|
| cold `cargo test --workspace --all-features --no-run` | 598 s | **166 s** (-72%) | at most +10% |
| `target/` | 28 GB | **11 GB** (39%) | at most 50% |
| test executables | 147 (19.0 GB) | **21** (2.7 GB) | 20 |
| incremental | 6.5 GB | 5.5 GB | - |
| warm rebuild after touching `htui/tests/backlog.rs` | 37 s | 4 s | - |
| warm rebuild after touching `htui-core/tests/prompt_render.rs` | 3 s | 1 s | - |

Both cold builds compiled the same 528 units. The 21st executable is the `panic_hook` exemption, added after D8 set
the bound at 20 (11 unit + 8 `it` + `extensibility`); accepted. The cold time fell instead of rising: the plan's
risk (one big crate links serially) did not happen, because 136 links of the whole tree were the cost.

Test names: `names.sh` (`--list` and `--list --ignored`, prefixed with the old file name) is identical to the
base for every crate (8,508 names).

**What an item run costs now** (one crate's `it` binary, default libtest threads, live Postgres, Qdrant skipped):

| crate | default threads | shuffled (seed 11) | base, every old binary at `--test-threads=1` |
|---|---|---|---|
| htui-store | 1022 s | 949 s | 2992 s |
| htui | 122 s | 154 s | 1395 s |
| htui-agent | 1.2 s | 1.5 s | 40 s |

The base column is the T0 baseline run, single-threaded, so it is not a like-for-like comparison: it bounds what an
item run cost when agents ran the crate's tests the safe way. htui-store's wall time is one test:
`pg_conformance::pg_store_conformance` runs every conformance case in sequence and finishes last, at about 1020 s
(its old 3-test binary took 971 s alone). Every other htui-store test finishes well before it. Splitting that test is
the next lever for htui-store item runs; it is not a layout problem.

Order and parallelism: the parallel and shuffled runs failed on the same tests, and none of them is an order
dependency. `claude_plugin::the_emitted_plugin_validates_and_the_mod_tests_pass_under_claude` is a base failure
(`claude plugin validate --strict` from the installed `claude` 2.1.323 fails the emitted plugin), and
`launch::spawn_runs_the_resolved_command_under_supervision` reads `$CARGO`, which `after.sh` does not set because it
runs the binary directly; under `cargo test` it passes. One shuffle seed was run, not the plan's three: the
maintainer ruled out hour-long runs.

## Gate

`bash scripts/gate.sh` on `fe802740` with the host Postgres DSN (Qdrant unset): 30 min, every step green but
`test` (1732 s). Its four failures:

- base failures, unchanged: `claude_plugin::the_emitted_plugin_validates_and_the_mod_tests_pass_under_claude` and
  `worker_pg::the_worker_binary_never_exposes_its_dsn`;
- load flakes in htui-agent: `cli_driver::a_cancel_drains_a_late_result_and_reports_its_done_rather_than_a_second_one`
  and `launch::an_interrupt_lets_the_child_exit_on_its_own_terms`. Both wait on a fixed window. The `it` binary
  took 36.7 s in the gate; run alone with `--test-threads=1` it passed twice in 12.5 s, and both passed in the
  default-thread and shuffled runs. The same two failed in the gate on `bd979072`, so they are a load-sensitive
  pair worth a wider window, not an order dependency.

Where the test step's 1732 s go, single-threaded: htui-store `it` 928 s, htui `it` 376 s, htui unit tests 156 s,
htui-mcp 70 s, htui-agent 50 s, the rest under a minute together.

## Lessons

- **The plan's audit missed two dependencies on one file per binary.** It covered `tests/` and the `src/` statics,
  but not `src/` code that reads a test file by path (`htui-core/src/store/conformance.rs` cross-references
  `htui-store/tests/{pg_criteria,box_identity}.rs`, found by the gate) or a test that calls `set_hook` (found by the
  Opus review). Audit a layout move for path reads (`include_str!`, `read_to_string`, `concat!(env!("CARGO_MANIFEST_DIR"), ...)`)
  in `src/` as well as `tests/`, and for every process-wide setter (`set_hook`, `set_var`, `set_current_dir`, global
  subscribers).
- **A crash now ends the crate's whole `it` binary.** A stack overflow or SIGABRT in one test used to cost one small
  binary; it now hides every later result in that crate. `abort-grep` in the gate names it, and `AGENTS.md` says so.
- **Item runs filter by module, never by crate.** `--test it <module>::` at default threads is the item-sized run;
  the full gate runs once, at return.

## Owed

- Windows: `scripts/gate.ps1` was not run; the moves are platform-neutral and the check stays with MOD-16.
- Sandbox runs in flight on the old layout (MOD-125 M3, MOD-152 and others) collect onto renamed files: git follows
  the renames for edited files, and a new file at the old `tests/<name>.rs` path becomes its own target until moved.
