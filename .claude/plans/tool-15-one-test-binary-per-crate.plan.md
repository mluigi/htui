# Plan: TOOL-15 — One integration-test binary per crate

**Source**: `HANDOFF.md` TOOL-15 (from the 2026-11-13 hr build-cache trial). Routed as **plan** by maintainer
override (C3 and C4 fired, which routes to PRD; the maintainer declined the PRD on 2026-11-13).
**Complexity**: Medium-Large. The change is mechanical (moves, a `main.rs` per crate, snapshot renames) but wide:
136 files, 270 snapshots and about 2,940 tests. The risk sits in the few places that depended on one process or
one file per test target.

## Summary

Each file under `crates/*/tests/` is its own test target, so cargo links 136 executables of 130-340 MB against
the same dependency tree (19.8 GB of a 29 GB `target/`, 7m33s cold at 4 cpus). Cargo builds one executable per
`tests/*.rs` and per `tests/<dir>/main.rs`, so each crate gets `tests/it/main.rs` and its old files become
`tests/it/<old-name>.rs` modules: 136 integration executables become 8 `it` binaries plus one exempted scan test.
No `Cargo.toml` changes. The plan answers the four questions the item left to it (D3 to D6), keeps every
invariant checkable by a before/after test-name diff, and ends with a measured before/after.

## Decisions

**D1 — Layout.** `crates/<c>/tests/it/main.rs` declares `mod <old-name>;` for every old file; each old file moves
with `git mv` to `tests/it/<old-name>.rs`, content unchanged unless a task says otherwise. Fixture directories
(`fixtures/`, `transcripts/`) and non-test data stay where they are. The target is named `it` in every crate, so
the replacement for `cargo test --test backlog` is `cargo test --test it backlog::` (a libtest filter on the
module path). Directory modules keep their shape: `backlog_keys.rs` + `backlog_keys/` moves as a pair, and
`token_scan/mod.rs` (declared by two files) and `support/mod.rs` become a single `mod` in `main.rs` that the
users reach as `crate::token_scan` / `crate::support`.

**D2 — One exemption: `crates/htui-agent/tests/extensibility.rs` stays a top-level target.** It is not std-only
(it imports `htui_agent::{conformance, fake, record, ...}`, so it does link the crate), but its path is
hard-coded in two places that a move would break: its own self-exclusion (`tests/extensibility.rs:110`) and the
`htui-dev` mod's guard (`.claude/skills/htui-dev/hooks/guards.ts`, `slice(-4)` compare, and its tests). Moving it
would mean editing the mod and restarting Claude Code. One extra executable is the cheaper carve-out; a later
item can move it with the mod change.

**D3 — Process-wide state (item question a): keep per-test guards, no `cargo-nextest`.** The audit found the
state is already isolated without a process per file:
- the keyring fake is behind `htui_store::testkit::mock_keyring()`, whose guard takes a process-wide async lock,
  resets the slot on install and sets it back to `None` on drop (`htui-store/src/testkit.rs:318-340`);
- there is no environment mutation: the workspace forbids `unsafe`, so `set_var` is unavailable; the only
  `set_current_dir` is `chat_live_agy.rs` (an `#[ignore]` live test with a `RestoreCwd` drop guard);
- a case that needs a clean process re-runs the test binary as a child with `--exact <name>`: five sites in four
  files (`launch.rs` ×2, `cli_live.rs`, `infisical.rs`, `worker_crash_pg.rs`). **These break on a merge** because
  the libtest name of a top-level fn is now `launch::<fn>`, not `<fn>`; task owners fix the name (D7);
- the per-process statics in `src/` (`WARNED` in `htui-orch/src/graph.rs`, the kill-point `ARMED`, the git `DECIDED`
  probe, `shared_qdrant`, the rustls `Once`) already share one process in the 12 unit-test executables, and none of
  them is asserted from `tests/` except `ARMED`, which only the child process of `worker_crash_pg.rs` reads.
`nextest` would restore process isolation exactly but costs a gate-runner change, a separate doctest step, a tool
install in the hr image and a spawn per test (about 2,940), for a hazard the audit did not find. It stays the
fallback if T6's shuffled run finds an order dependency that a guard cannot fix.

**D4 — Insta renames (item question b).** Probe on insta 1.48.0 (the repo's version): a merged binary writes
snapshots to `tests/it/snapshots/it__<file>__<name>.snap`, nested modules to `it__<file>__<mod>__<name>.snap`,
unnamed ones to `it__<file>__<fn>.snap`; a stale `source:` header does not fail a comparison, a wrong filename
does (and writes `.snap.new`). So each task renames, never regenerates: every `snapshots/` directory that sat next
to a moved file moves next to its new location (`tests/snapshots` → `tests/it/snapshots`,
`tests/<sub>/snapshots` → `tests/it/<sub>/snapshots`) and every `.snap` file gets the `it__` prefix. 270 files:
htui 229, htui-core 32, htui-agent 8, htui-mcp 1. Contents are never edited.

**D5 — Feature gating (item question d).** Most files open with `#![cfg(feature = "...")]`, which still works as
an inner attribute of an out-of-line module, but then a merged binary compiled without the feature contains only
the ungated modules and reports ok. Each crate with gated modules gets a canary at the root of `main.rs`:
```rust
#[cfg(not(feature = "testkit"))]
#[test]
fn the_testkit_feature_is_off_so_most_of_this_binary_is_not_compiled() {
    panic!("run with --features testkit (or --all-features): the gated modules compile to nothing without it");
}
```
(`testkit` for htui, 54 of 68 files gated; `test-support` for htui-core, 11 of 12, and htui-agent, 4 of 28.)
`htui-store` needs none: its `dev-dependencies` enable its own `test-support`, which implies `demo`. A canary test
rather than `compile_error!`, so `cargo check --tests` and rust-analyzer stay clean without features. Consequence
to state in the close-out: a featureless `cargo test -p htui` now goes red where it used to pass with 0 tests.

**D6 — One binary per crate, not per workspace (item question c).** `CARGO_BIN_EXE_htui` (`keys_file.rs`,
`mcp_stdio.rs`, `worker_pg.rs`) is set only for integration tests of the package that owns the `htui` bin, so those
files must stay in `crates/htui`. A single workspace-wide test crate would also need every crate's test-only
surface re-exported and would lose per-crate incremental builds. Per crate it is.

**D7 — Mechanical edits the move forces** (each task applies the ones its crate has):
1. `include_str!` paths are relative to the file: add one `../` (`fixtures/x` → `../fixtures/x`, `../src/x` →
   `../../src/x`, `../migrations/x` → `../../migrations/x`). Seven files, about 20 sites (T-owner list below).
2. Self re-exec names: build the child's `--exact` argument from the module path, e.g.
   `let name = format!("{}::{FN}", module_path!().split_once("::").map_or("", |(_, rest)| rest))` (libtest drops the
   crate name), and keep each parent's "the child ran exactly one test" assertion: `launch.rs:1343,1441` and
   `worker_crash_pg.rs:746` and `cli_live.rs:2076` already assert it; **`infisical.rs:1633` does not** (it checks
   only the exit status), so T1 adds a `"1 passed"` assertion there. A child that matches nothing must fail the
   parent, never pass it.
3. `mod token_scan;` ×2 (`kit_chrome.rs`, `no_hardcoded_keys.rs`) and `mod support;` (`infisical.rs`) become a
   `use crate::...` of the root-declared module. `#[path = "backlog_keys/l_a.rs"]` ×3 stays valid (a `#[path]` in a
   non-`mod.rs` file resolves from that file's directory).
4. The live runbook headers (`--test auth_live ... -- --ignored`) become `--test it auth_live:: -- --ignored`, and
   the four `ONE_AT_A_TIME` live mutexes in htui-agent no longer exclude each other across modules, so the headers
   also say "run one module at a time, `--test-threads=1`" (they were separate processes before).

**D8 — Acceptance numbers.** Measured on the host with `CARGO_BUILD_JOBS=4` and a fresh `CARGO_TARGET_DIR`, same
recipe before and after (the 7m33s / 29 GB figures came from the hr image, so only the A/B ratio counts):
test executables 147 (measured on the base: 136 integration + 11 unit, 19.0 GB; the item's 148 counted one more) →
at most 20 (11 unit-test + 8 `it` + `extensibility`); `target/` size at most 50% of the base (28 GB on the host);
cold `--no-run` wall time at most +10% of the base. If the cold time exceeds that (one big crate compiles
serially, where 68 small targets compiled in parallel), split the largest crate's `it` into at most three
area binaries (`ui`, `pg`, `chat` for htui) chosen from the numbers, not now.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Module-per-file under one root | `crates/htui/tests/backlog_keys.rs` (+ `backlog_keys/l_a.rs`) | a test file that declares one module per lane; its `//!` doc states why |
| Shared test support | `crates/htui-secrets/tests/support/mod.rs`, `crates/htui/tests/token_scan/mod.rs` | helper module used by several files |
| Process guard that resets | `crates/htui-store/src/testkit.rs:334-340` (`mock_keyring`, `KeyringGuard::drop`) | lock + reset on install + reset on drop |
| Child re-exec with a loud failure | `crates/htui/tests/worker_crash_pg.rs:86,746` (H-14) | `--exact <name>` plus an assertion that the child ran |
| Gate commands | `scripts/gate.sh` `step_test`: `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` | the verification the merged binaries must keep green |

## Files to Change

| Task | Files (all under `crates/<crate>/tests/` unless noted) | Action |
|---|---|---|
| T0 | `.claude/plans/tool-15-one-test-binary-per-crate.tools/{names.sh,measure.sh}`; scratch under `/media/projects/tool-15/` (outside the repo) | CREATE |
| T1 | `htui-secrets/` (2 files + `support/`), `htui-worker/` (2), `htui-orch/` (5), `htui-mcp/` (6, 1 snapshot, 2 `include_str!`) | MOVE, `main.rs` CREATE |
| T2 | `htui-core/` (12 files, 32 snapshots, `prompt_digest.rs` 12 `include_str!`, canary) | MOVE, `main.rs` CREATE |
| T3 | `htui-store/` (13 files, `migrations.rs` 1 `include_str!`) | MOVE, `main.rs` CREATE |
| T4 | `htui-agent/` (28 files minus `extensibility.rs`, 8 snapshots, `launch.rs` ×2 and `cli_live.rs` re-exec, `estimator_live.rs` 2 `include_str!`, 8 live headers, canary) | MOVE, `main.rs` CREATE |
| T5 | `htui/` (68 files, 229 snapshots, `worker_crash_pg.rs` re-exec, `token_scan/`, 2 `include_str!`, 3 live headers, canary) | MOVE, `main.rs` CREATE |
| T6 | `README.md` (line 705), `docs/htui-secrets.md` (610), `run_loop.sh` (13), `AGENTS.md` (new short section) | UPDATE |

Task independence by file-set intersection: T1-T5 each own a different `crates/<c>/tests/` tree and T6 owns four
files outside them, so the sets are pairwise disjoint (checked in the claims table). They still **run serially on
one checkout**: they share `target/` (cargo's build lock) and `.git/index`, and a parallel lane would need a
worktree and about 10 GB of `target/` each. No file is edited by two tasks.

## Tasks

Each brief cut from this plan carries D1-D8 plus its own task section. Every task checks with the commands below,
never the workspace gate. `<names>` is `names.sh` from T0 (it lists `--list` and `--list --ignored` for the crate,
prefixes each name with the old test-file name, and sorts), `<base>` is T0's saved list for the crate.

### Task 0: Baseline, tools and measurement
- **Action**: create a detached worktree of `ae910789` at `/media/projects/tool-15/base` with its own
  `CARGO_TARGET_DIR`; write `names.sh` and `measure.sh` (the latter: cold `cargo test --workspace --all-features
  --no-run` under `/usr/bin/time -v` at `CARGO_BUILD_JOBS=4`, then `du -sh target`, executable count and total
  size under `target/debug/deps`, `target/debug/incremental` size, then a warm rebuild after `touch
  crates/htui/tests/backlog.rs` and after `touch crates/htui-core/tests/prompt_render.rs`); run the base
  measurement; save `names.sh` output per crate as `<base>`, and the per-crate `test result:` totals of one
  `cargo test -p <crate> --all-features --no-fail-fast -- --test-threads=1` run per crate (with the host DSN
  exported the way the gate does, so Postgres suites run live).
- **Validate**: `<base>` lists sum to 2,940 ± the `#[test]` count from the claims table; the measurement log holds
  wall time, size, executable count, incremental size. Nothing in the repo changes except the two scripts.

### Task 1: htui-secrets, htui-worker, htui-orch, htui-mcp (the pilot; a foundation task, so it gets the verifier)
- **Action**: apply D1, D4 (mcp: 1 snapshot), D7.1 (mcp `transcripts/` ×2), D7.2 (`infisical.rs`), D7.3
  (`infisical.rs` `support`). `htui-worker`'s and `htui-orch`'s files are plain. No canary (no gated files except
  two `#![cfg(unix)]` in mcp). The recipe is proven here; later tasks copy it.
- **Validate**: for each of the four crates: `<names>` diff against `<base>` is empty (both lists);
  `INSTA_UPDATE=no cargo test -p <crate> --all-features --no-fail-fast -- --test-threads=1` gives the same
  passed/ignored totals as the base and no `*.snap.new`; `cargo clippy -p <crate> --all-features --all-targets -- -D warnings`;
  `cargo fmt --check`.

### Task 2: htui-core
- **Action**: D1, D4 (32 snapshots), D5 (`test-support` canary; 11 of 12 files gated), D7.1 (`prompt_digest.rs`:
  12 sites `../src/` → `../../src/`).
- **Validate**: as Task 1 for `htui-core`; plus `cargo test -p htui-core` featureless fails on the canary alone.

### Task 3: htui-store
- **Action**: D1, D7.1 (`migrations.rs`: `../migrations/0023_...` → `../../migrations/...`). No snapshots, no canary.
  Several files are Postgres suites: run them with the host DSN.
- **Validate**: as Task 1 for `htui-store`, DSN exported so `pg_*`, `cache`, `migrations` run live.

### Task 4: htui-agent
- **Action**: D1 (28 files; `extensibility.rs` stays, D2), D4 (8 snapshots), D5 (`test-support` canary), D7.1
  (`estimator_live.rs`: `fixtures/estimator_*.txt`), D7.2 (`launch.rs:1328,1426`, `cli_live.rs:2045`), D7.4 (8
  live headers).
- **Validate**: as Task 1 for `htui-agent`; the two `launch.rs` re-exec cases and the `cli_live` case (`--ignored`
  runs its child under `--exact`) pass and print "1 passed" from the child; `extensibility` still runs as its own target.

### Task 5: htui (the largest; a foundation task for nothing, so no verifier, but its fix round is verified)
- **Action**: D1 (68 files), D4 (229 snapshots, including `backlog_keys/` sub-directory snapshots if any),
  D5 (`testkit` canary), D6 (`keys_file.rs`, `mcp_stdio.rs`, `worker_pg.rs` keep `env!("CARGO_BIN_EXE_htui")`),
  D7.1 (`prompt_settings.rs`, `panic_hook_order.rs`: `../src/` → `../../src/`), D7.2 (`worker_crash_pg.rs:86,867`),
  D7.3 (`kit_chrome.rs`, `no_hardcoded_keys.rs` share one `token_scan`), D7.4 (3 live headers).
- **Validate**: as Task 1 for `htui`, with `--all-features`, the host DSN and `HTUI_TEST_QDRANT_URL` as the gate has them;
  `cargo test -p htui --all-features --test it worker_crash_pg:: -- --test-threads=1` explicitly (the child case).

### Task 6: Docs, the shuffled run, the after-measurement
- **Action**: update the four doc files (the new invocation, the module-path filter, the featureless canary note;
  `AGENTS.md` gets a short "Integration tests: one binary per crate" section next to the gate rules); run
  `measure.sh` on the result in a fresh target dir; run one order-independence check: a separate target dir with
  `RUSTC_BOOTSTRAP=1`, `cargo test -p <crate> --all-features --test it -- -Z unstable-options --shuffle --test-threads=1`
  with three seeds for htui, htui-agent and htui-store (stable rejects `-Z`; the probe confirmed the flag works
  with the bootstrap variable). Compare against D8 and write the numbers into the write-up.
- **Validate**: the doc grep finds no `--test <old-file>` outside archived plans, `docs/decisions/` and
  `docs/benchmarks/` (history stays as written); D8 numbers recorded; shuffled runs green or the order dependency
  filed.

## Validation

```bash
bash .claude/plans/tool-15-one-test-binary-per-crate.tools/names.sh <crate> | diff - <base>/<crate>.names     # per task
INSTA_UPDATE=no cargo test -p <crate> --all-features --no-fail-fast -- --test-threads=1                       # per task
cargo clippy --workspace --all-features -- -D warnings && cargo clippy --workspace -- -D warnings              # featureless too
bash scripts/gate.sh                                                                                           # once, main thread, after T5
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh                                              # close-out
```

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| 136 integration files: htui 68, htui-agent 28, htui-store 13, htui-core 12, htui-mcp 6, htui-orch 5, htui-secrets 2, htui-worker 2 (htui-bench 0) | confirmed | `find crates/*/tests -maxdepth 1 -name '*.rs'` |
| No `[[test]]` section and no `autotests` in any crate `Cargo.toml`, so `tests/it/main.rs` is auto-discovered | confirmed | `grep` of all crate manifests (only `[[bin]]` in htui); the probe crate built and ran `it` |
| Test count about 2,940: htui 1,572, agent 634, store 379, core 174, mcp 76, orch 72, secrets 72, worker 35 | confirmed (counts `#[test]`/`#[tokio::test]` lines, not parametrised cases) | `grep -rE '^\s*#\[(tokio::)?test'` |
| Snapshots under `tests/`: 270 (htui 229, core 32, agent 8, mcp 1); 69 more under `src/` are untouched | confirmed | `find ... -name '*.snap'` |
| insta 1.48.0 names a merged-binary snapshot `it__<file>__<name>.snap` in `tests/it/snapshots`, nested modules `it__<file>__<mod>__<name>` | confirmed by compile probe on the repo's toolchain (cargo 1.100.0, insta =1.48.0) | scratch crate, `INSTA_UPDATE=always` |
| A stale `source:` header does not fail a comparison; a wrong filename does and writes `.snap.new` | confirmed by probe | cases A and B of the probe |
| Stable libtest rejects `-Z unstable-options --shuffle`; `RUSTC_BOOTSTRAP=1` at build time enables it | confirmed by probe | probe cases C and D |
| The keyring fake is lock-guarded and reset on install and drop | confirmed | `mock_keyring`, `KeyringGuard::drop` source |
| No test mutates the environment; `set_current_dir` only in `chat_live_agy.rs` | confirmed | `set_var(` matches one fn name only; `unsafe` matches are comments; `set_current_dir` 3 hits, one file |
| Five self re-exec sites, four files, three crates | confirmed | `current_exe` hits: `launch.rs:1330,1428`, `cli_live.rs:2052`, `infisical.rs:1607`, `worker_crash_pg.rs:865` |
| `launch.rs` parents already assert "1 passed" from the child; `worker_crash_pg.rs` has the H-14 check | confirmed | `launch.rs:1343,1441`; `worker_crash_pg.rs:746` |
| `cli_live.rs` parent asserts the child ran one test | confirmed | `cli_live.rs:2076` (`"test result: ok. 1 passed"`) |
| **`infisical.rs` parent does NOT assert it** (only `status.success()`, `infisical.rs:1633`): after the move a mistyped `--exact` name would pass vacuously | **falsified premise, fix added** | T1 adds the `"1 passed"` assertion (D7.2) |
| `#![cfg(...)]` is allowed at the top of an out-of-line module file, and a gated-out module compiles to nothing | confirmed by compile probe (module with `#![cfg(feature = "nope")]` beside live modules: 3 tests ran, the gated one absent) | 84 `#![cfg` and 7 `#![expect` inner attributes in test files |
| `htui-core` does not enable its own `test-support` in dev-dependencies; `htui-store` does | confirmed | `htui-core/Cargo.toml:32-37`; `htui-store/Cargo.toml:60-61` |
| `include_str!` sites: 7 files (`estimator_live`, `prompt_digest`, mcp `protocol`, mcp `tools_permission`, `migrations`, `prompt_settings`, `panic_hook_order`) | confirmed | `grep -rnE 'include_(str\|bytes)!'` |
| `#[path]` only in `backlog_keys.rs`; `mod token_scan;` ×2; `mod support;` ×1 | confirmed | `grep` of `mod` declarations in test files |
| No `crate::` path in test code (2 hits are doc comments) | confirmed | `cli_conformance.rs:14`, `install_live.rs:22` are `//!` lines |
| `extensibility.rs` is not std-only and its path is hard-coded in the `htui-dev` mod | confirmed | its `use htui_agent::...` lines; `guards.ts:874,920,928,977,981`; self path at `extensibility.rs:110` |
| About 813 `--test <name>` mentions in md/sh/js/ts; live ones are 11 runbook headers, `README.md:705`, `docs/htui-secrets.md:610`, `run_loop.sh:13` | confirmed (the rest sit in archived plans, `docs/decisions/`, `docs/benchmarks/`, the mod's own test samples) | `grep -rn -- '--test [a-z_0-9]+'` |
| `htui-dev` needs no change: it guards `cargo test` without `testkit`, and its `--test app` strings are samples | confirmed from `cargo.test.ts` / `guards.ts:265` | read |
| T1-T5 touch pairwise disjoint file sets; none edits a `Cargo.toml` | confirmed by the table above (each owns one `crates/<c>/tests/` tree) | file-set intersection is empty |
| No other worktree or `hr/*` branch exists in this repo | confirmed | `git worktree list`, `git branch --list 'hr/*'` |
| The hr image's 7m33s / 29 GB baseline reproduces on the host | **open** | T0 measures base and result with one recipe; only the ratio is used |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| One big crate compiles serially: cold wall time rises although total disk falls | Medium | D8 threshold and the split fallback; T0 and T6 time cold and warm builds |
| A module silently drops out (cfg, typo in `main.rs`) | Medium | `<names>` before/after diff per task, `--list --ignored` too; the canary for gated crates |
| A re-exec child matches no test and the parent passes | Medium | D7.2: every parent asserts the child ran one test; T4/T5 run those cases by name |
| Order dependence between files that never shared a process | Low-Medium | T6 shuffled runs (3 seeds) on htui, agent, store; per-test guards already exist for the keyring |
| Missed snapshot rename | Low | `INSTA_UPDATE=no`, no `.snap.new`, snapshot count equal before and after |
| Sandbox runs in flight (e.g. MOD-125 M3, MOD-152) edit `tests/*.rs` on the old layout | Medium | git follows the renames at `hr collect` for edited files; a file such a run adds at the old path becomes a separate target (builds and runs, just not merged) |
| Windows (`.ps1` gate twin, `#![cfg(unix)]` modules) cannot be run here | Low | the moves are platform-neutral; Windows verification stays with MOD-16 |
| Featureless `cargo test -p htui` now fails on the canary | Certain, intended | D5; stated in `AGENTS.md` and the write-up |

## Acceptance

- [ ] 136 files in 8 `it` binaries (+ `extensibility`); `<names>` lists (normal and `--ignored`) identical to the base for every crate
- [ ] 270 snapshots renamed, none regenerated; `INSTA_UPDATE=no` green, no `.snap.new`
- [ ] The five self re-exec cases run their child and fail the parent when the child ran nothing
- [ ] Canary present in htui, htui-core, htui-agent; featureless run goes red on it alone
- [ ] `bash scripts/gate.sh` green on the merged tree, both clippy runs clean
- [ ] D8 numbers measured and recorded in the write-up (executables, `target/` size, cold and warm times)
- [ ] Live runbooks, `README.md`, `docs/htui-secrets.md`, `run_loop.sh` and `AGENTS.md` use the new invocation
- [ ] Validator green at close-out
