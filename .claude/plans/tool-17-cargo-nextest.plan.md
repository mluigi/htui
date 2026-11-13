# Plan: TOOL-17 — cargo-nextest as the gate's test runner

**Source**: `HANDOFF.md` TOOL-17 (from TOOL-16). Routed as **plan** (C3 fired: adoption waits on a measured
spike; C1, C2, C4 not). Branch `tool-17`, primary checkout.
**Complexity**: Small-Medium. Ten files, no product code. The risk is in the measurement and in tests that
only ever ran beside each other in one process.

## Summary

`cargo test --workspace` runs test binaries one after another: htui's unit tests (95 s), htui's `it` (87 s) and
htui-store's `it` (221 s) never overlap. nextest runs every test of every binary in one pool, one process per
test. This plan starts with a **measured spike** that times nextest against `cargo test` on the same tree in the
same session, then stops at a **go/no-go gate** for the maintainer. Only on "go" do both gate twins move to
nextest (plus a separate doctest run) and does the hr image get the binary.

## Decisions

**D1 — Spike first, timed by the main thread.** Agents never time anything (memory: long cargo runs stay out of
agents; one runner times sequentially). The main thread prebuilds the test binaries and doctests, then times
on the same tree in the same session:
(a) `cargo test --workspace --all-features --no-fail-fast` (today's test step);
(b) `cargo nextest run --workspace --all-features --no-fail-fast --profile gate`;
(c) `cargo test --workspace --all-features --doc`.
Both have Postgres and Qdrant live. (b)+(c) against (a) is the comparison; TOOL-16's 457 s test step / 532 s with
build is context, not the baseline, because the HEAD and load differ. The spike also records the failures on each
side, the Qdrant group's serial wall time, the 10 slowest tests (from the JUnit file), and the process count.

**D2 — Go/no-go criterion.** **Go** when (b)+(c) wall time ≤ (a) wall time, and every failure in (b) that is
not in (a) is explained: either a real isolation bug that the fix round can repair, or filed as an item.
Otherwise **no-go**: write up the numbers, keep `cargo test`, and close TOOL-17 with the measurement. Either way
the maintainer decides at the gate; the numbers go into `docs/decisions/tool/tool-17.md`.

**D3 — Pinned prebuilt binary, cargo-nextest 0.9.151** (latest, released 2026-11-02). Host:
`curl -LsSf https://get.nexte.st/0.9.151/linux | tar zxf - -C ~/.cargo/bin`. Image: the same URL into
`/opt/rust/cargo/bin`, which `compose.hr.yaml` already puts on `PATH`; add `hr.cargo-nextest=0.9.151` to the
`LABEL`. Windows (gate.ps1's install hint): `https://get.nexte.st/0.9.151/windows-tar`.

**D4 — The Qdrant group is a name convention.** In `.config/nextest.toml`:

```toml
[test-groups]
live-qdrant = { max-threads = 1 }

[[profile.default.overrides]]
filter = 'test(/^qdrant_live::|^qdrant_worker::|live_qdrant/)'
test-group = 'live-qdrant'
```

All of `qdrant_live::` (htui-store `it`) and `qdrant_worker::` (htui `it`) go in. The six other cases that reach
the test Qdrant get a `live_qdrant_` prefix on their function name:

- `state_cli_pg`: `the_env_url_wins_over_the_row`, `a_search_without_a_collection_is_empty_and_loads_no_model`,
  `a_search_over_an_index_answers_in_scope`;
- `htui/src/concepts_worker.rs` tests: `a_tui_search_against_an_empty_qdrant_creates_no_collection_and_loads_nothing`,
  `a_tui_search_of_another_embedders_collection_is_refused_before_the_model`;
- `htui-bench/src/run.rs` tests: `run_with_wires_a_tiny_corpus`. It holds no `qdrant_turn` today because, as its own
  binary under `cargo test`, it never overlapped another Qdrant case. Under nextest it would.

An explicit list of names would rot silently when a case is added. Under the convention, the rule for a new
case is one line in AGENTS.md: "a live Qdrant test lives in `qdrant_live`/`qdrant_worker` or its name starts
with `live_qdrant_`". `cargo nextest show-config test-groups` lists what the group holds.

**D5 — `qdrant_turn` and `mock_keyring` stay** (a deviation from the item's "instead of `qdrant_turn`").
`cargo test -p <crate> --all-features --test it <module>::` remains the documented item-run command, and under it
the tests still share one process. Under nextest the locks cost nothing (one test per process). Their doc comments
say both runners. `qdrant_connect_to`'s retry stays too: it is about load, not overlap.

**D6 — Gate shape: the step list does not change.** The `test` step runs (b) then (c) into the same
`target/gate/test.log`, and it is red if either is. `STEPS`, `last.json`, the `htui-dev` mod's `GATE_STEPS` and the
self-test's step check are untouched. `abort-grep` stays: nextest prints `SIGABRT`/the overflow text for a crashed
test, and it still covers the doctest run. A missing `cargo-nextest` **fails** the test step with the install line
(D3). There is no silent fallback to `cargo test`, because "a green means the same thing everywhere" (gate.sh
header). The self-test gains one case: with `cargo-nextest` absent from `PATH`, the test step fails without
running anything.

**D7 — Profile `gate`**: `fail-fast = false`, `retries = 1` (nextest reports a pass on retry as `FLAKY` in the
summary line: the gate stays green and the flake is named), and JUnit at `target/nextest/gate/junit.xml` (per-test
times). The default profile has no retries, so a developer's own run shows failures as they are.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Gate step | `scripts/gate.sh:135-142` / `gate.ps1:134-141` | `tee` + `PIPESTATUS[0]` / `Tee-Object` + `$LASTEXITCODE`; comment names the item |
| Gate self-test | `scripts/gate.sh` `run_self_test` (`( … ) \|\| failures+=(…)` subshell cases) | one subshell case per behaviour, same in the ps1 twin |
| Image pins | `docker/hr/Dockerfile:55-61` | one `RUN` per tool group, version in a comment, `LABEL hr.<tool>=<ver>` |
| Testkit docs | `crates/htui-store/src/testkit.rs:462-477` | doc comment cites the item (`TOOL-16`) and says why |
| Test names | `crates/htui/tests/it/qdrant_worker.rs` | snake_case sentence names; module path is the libtest name |

## Files to Change

| File | Action | Task |
|---|---|---|
| `.config/nextest.toml` | CREATE | T1 |
| `crates/htui/tests/it/state_cli_pg.rs` | UPDATE (3 renames) | T1 |
| `crates/htui/src/concepts_worker.rs` | UPDATE (2 renames) | T1 |
| `crates/htui-bench/src/run.rs` | UPDATE (1 rename) | T1 |
| `scripts/gate.sh`, `scripts/gate.ps1` | UPDATE | T2 |
| `docker/hr/Dockerfile`, `docs/hr-sandbox.md` | UPDATE | T3 |
| `AGENTS.md`, `crates/htui-store/src/testkit.rs` (doc comments only) | UPDATE | T3 |

T1 ∩ T2 ∩ T3 = ∅ (checked below). They still run serially, because S (spike) sits between T1 and T2.

## Tasks

### T1: Qdrant test group (implementer)
- **Action**: create `.config/nextest.toml` exactly as D4 + D7 (default-profile override, `[profile.gate]` with
  `fail-fast = false`, `retries = 1`, `[profile.gate.junit] path = "junit.xml"`). Rename the six functions in D4 by
  prefixing `live_qdrant_`. Change nothing else in them.
- **Tests**: no new tests. The renames are the change.
- **Validate**: `cargo test -p htui --all-features --test it state_cli_pg::live_qdrant_`,
  `cargo test -p htui --all-features --lib concepts_worker::tests::live_qdrant_`,
  `cargo test -p htui-bench --all-features --lib live_qdrant_` (each runs the renamed cases);
  `cargo nextest show-config test-groups --all-features` lists all 32 `qdrant_live::`, the `qdrant_worker::` cases and the six;
  `cargo clippy -p htui -p htui-bench --all-features --all-targets -- -D warnings`; `cargo fmt --check`.

### S: Spike (main thread, not an agent)
- Install 0.9.151 on the host (D3). Run `cargo build --workspace --all-features --tests` and
  `cargo test --workspace --all-features --doc --no-run` first. Then time (a), (b), (c) of D1 one after another,
  each with `/usr/bin/time -v` and output to `/tmp/tool17/{a,b,c}.log`. Check `df -h /` before starting
  (memory: the dev Postgres crashes under disk pressure) and look for `57P03` storms afterwards.
- Write the numbers and failures into the plan's "Spike result" section, then **stop at the go/no-go gate (D2)**.

### T2: Gate twins (implementer, after "go")
- **Action**: `step_test` / `Step-Test` per D6. First check `command -v cargo-nextest` (ps1: `Get-Command
  cargo-nextest`); when it is missing, print `test: cargo-nextest not installed - <install line from D3>` and
  return fail. Otherwise run (b) piped into `tee "$TEST_LOG"`, then (c) piped into `tee -a`, and fail if either
  exit status is non-zero. Update the header comment ("whole workspace test suite at libtest's default
  threads" → nextest + doctests, TOOL-17) and the step comment. Add the D6 self-test case to both twins.
- **Validate**: `bash scripts/gate.sh --self-test`; `pwsh -NoProfile -File scripts/gate.ps1 -SelfTest`; `shellcheck scripts/gate.sh` if installed. The full gate is the main
  thread's, not the agent's.

### T3: Image and docs (implementer, after "go")
- **Action**: in `docker/hr/Dockerfile`, add a `RUN` that downloads the pinned prebuilt into
  `/opt/rust/cargo/bin`, and add `hr.cargo-nextest=0.9.151` to the `LABEL`. In `docs/hr-sandbox.md:10` and `:92`,
  add `cargo-nextest` to the toolchain lists and say that an existing image needs `scripts/hr build`. In AGENTS.md
  (Tests bullets at lines 35-56), say the gate runs nextest + `--doc`, give the item-run command for both runners
  (`cargo nextest run -p <crate> --all-features -E 'test(/^<module>::/)'`), state the `live_qdrant_` rule (D4),
  and state that `qdrant_turn`/`mock_keyring` stay for `cargo test` (D5). Update the doc comments on
  `testkit::qdrant_turn` and `QDRANT` (`testkit.rs:462-477`) to cover both runners. No code changes in testkit.
- **Validate**: `docker build` is **not** run by the agent; the main thread runs `scripts/hr build` once
  after merge, or leaves it to the host maintainer. `cargo doc -p htui-store --all-features --no-deps` stays
  warning-free; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Validation

Main thread after T2+T3: `bash scripts/gate.sh` (full; nextest-based test step). Expected red set: the three
TOOL-16 base failures, unless the spike showed otherwise. Then the `rust-reviewer` runs over
`git diff main..HEAD` with this plan.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Per-process setup (template lookup + sweep query, `concepts_worker` model load) × ~8,500 tests eats the overlap gain | Medium | That is what S measures; D2 says no-go if it does |
| A test that passed only because a neighbour in the same process set something up (or the reverse) | Low-Med | S lists (b)-only failures; D2 requires each one explained |
| ~12 test processes each with a Postgres pool exhaust `max_connections` | Low | Same parallelism as today's in-process threads; S watches for `53300` |
| A new live Qdrant test misses the group | Medium | D4 naming rule in AGENTS.md; `show-config test-groups` to check |
| Retries hide a real flake | Low | `FLAKY` stays in the summary line; the default profile has no retries |
| The hr image is stale until rebuilt, and the sandbox gate fails on a missing binary | Certain until rebuilt | D6's install line names the fix; T3's doc note; close-out lists `scripts/hr build` |

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `qdrant_turn` is defined in testkit and called from 4 files | ✓ | scan: `testkit.rs:475`; `qdrant_live.rs`, `qdrant_worker.rs`, `state_cli_pg.rs`, `concepts_worker.rs` |
| The six D4 cases are the only live-Qdrant tests outside `qdrant_live`/`qdrant_worker` | ✓ | scan of `HTUI_TEST_QDRANT_URL` across `crates/`: those two modules, `state_cli_pg` (3 via `test_qdrant`), `concepts_worker` (2), `htui-bench run.rs:1029` (1) |
| `htui-bench`'s live case holds no `qdrant_turn` | ✓ | `run.rs:1024-1033` reads the env directly |
| No module other than the two is named `qdrant_live`/`qdrant_worker` | ✓ | `mod` scan: the other qdrant modules are `qdrant_url_pg`, `qdrant_cli_pg`, `qdrant_settings(_pg)`, `qdrant_cmd`, `qdrant_settings_info`, `qdrant`, `qdrant_key_tests` |
| The template build is safe across processes | ✓ | `testkit.rs:68-71,173-191`: `pg_advisory_lock(TEMPLATE_LOCK)` around lookup/build |
| The sweep and the template lookup run once per **process**, so ~once per test under nextest | ✓ | `testkit.rs:63,66` `OnceCell` statics |
| No crate has a custom test harness (`harness = false`) that nextest might not list | ✓ | scan of every `crates/*/Cargo.toml` and the root: none |
| nextest 0.9.151 takes `--workspace --all-features --no-fail-fast --retries -P/--profile` | ✓ | `/tmp/nextest-probe/cargo-nextest nextest run --help` |
| `.config/nextest.toml` test group + regex filter assigns as D4 says; a custom profile inherits default overrides; JUnit lands at `target/nextest/<profile>/junit.xml` | ✓ | scratch crate probe: `show-config test-groups --profile gate` listed `qdrant_live::a`, `qdrant_worker::b`, `state_cli_pg::live_qdrant_c` and not `qdrant_url_pg::d`/`plain`; `target/nextest/gate/junit.xml` written |
| Pinned URLs resolve | ✓ | `get.nexte.st/0.9.151/linux` → 302 to the `cargo-nextest-0.9.151-x86_64-unknown-linux-gnu.tar.gz` release; `/windows-tar` → 302 |
| `/opt/rust/cargo/bin` is on the sandbox PATH | ✓ | `compose.hr.yaml:94` |
| Nothing in `scripts/` checks image labels, so the new label only documents | ✓ | scan for `hr.cargo-insta` / `hr.sqlx-cli` in `scripts/`: no hits |
| `htui-dev` mod's `GATE_STEPS` is tied to `STEPS`, so D6 keeps the list | ✓ | `gate.sh` self-test compares `GATE_STEPS` in `.claude/skills/htui-dev/hooks/commands.ts` |
| Test binaries that re-run themselves as a child (`--exact <name>`) work under nextest | ✓ | S: no (b)-only failure; the only failure is the `claude_plugin` base case |
| Host has pwsh for the ps1 self-test | ✓ | `/home/mluigi/.local/share/mise/shims/pwsh` |
| Task file sets are disjoint | ✓ | T1 {nextest.toml, state_cli_pg.rs, concepts_worker.rs, run.rs} · T2 {gate.sh, gate.ps1} · T3 {Dockerfile, hr-sandbox.md, AGENTS.md, testkit.rs}: pairwise ∅ |

## Spike result

HEAD `07f19f2f`, 2026-11-13 18:02, host (12 cpus, 60 GB), Postgres and Qdrant live, load average ~7 from other
work before the start. Prebuild 7 s (warm). Sequential, `/usr/bin/time -v`, logs in `/tmp/tool17/`:

| Run | Wall | Result |
|---|---|---|
| (a) `cargo test --workspace --all-features --no-fail-fast` | **381 s** | 8,701 passed, 1 failed, 44 ignored (doctests included) |
| (b) `cargo nextest run --workspace --all-features --no-fail-fast --profile gate` | **287 s** | 8,697 run: 8,696 passed, 1 failed (twice, retry included), 44 skipped; 4 slow, 1 leaky |
| (c) `cargo test --workspace --all-features --doc` | **4 s** | 5 passed |
| (b)+(c) against (a) | **291 s vs 381 s, -24 %** | test counts match: 8,697 + 5 = 8,702 = 8,701 + 1 |

- **Failures:** the same one on both sides, `claude_plugin::the_emitted_plugin_validates_and_the_mod_tests_pass_under_claude`
  (TOOL-16 base failure). `worker_pg` and `worker_crash_pg`, the other two base failures, passed in both. No
  (b)-only failure: the five self-re-running tests (TOOL-15 D3) pass under nextest. No `57P03`, no aborts.
- **Leaky:** `editor::tests::suspension::a_dropped_future_kills_the_editor` leaves a handle open past its exit
  (nextest marks it LEAK and still passes it).
- **Per-test (JUnit):** 2,513 s of test time in 287 s of wall time (8.8x). The long poles are the four
  `pg_conformance` shards at ~101 s each, then `keys::validate::…a_version_1_entry_never_takes_a_key…` at 56 s and
  `migrations::migration_0018…` at 39 s. The 40 `live-qdrant` cases sum to 156 s serially, which fits inside the
  run.
- **Verdict per D2: go**, since (b)+(c) ≤ (a) and there are no unexplained failures. The maintainer decides.

## Acceptance
- [x] T1, S done; go/no-go recorded (go)
- [x] On go: T2, T3 done; full gate red only on the known base failures; reviewer findings applied (`31ddad13`)
- [x] `docs/decisions/tool/tool-17.md` holds the measurement either way
