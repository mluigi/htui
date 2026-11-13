# TOOL-17 - cargo-nextest as the gate's test runner (done, 2026-11-13)

From TOOL-16. `cargo test --workspace` runs its test binaries one after another, so htui's unit tests, htui's
`it` and htui-store's `it` never overlap. nextest runs every test of every binary in one pool, one process per
test. Plan: `.claude/plans/tool-17-cargo-nextest.plan.md` (decisions D1-D7, verified-claims table, spike result).
Branch `tool-17` on `4ee9f174`, in the primary checkout.

## Spike (measured before adopting)

These runs were on the same tree (`07f19f2f`) in the same session, one after another, on the host (12 cpus) with
Postgres and Qdrant live:

| Run | Wall | Result |
|---|---|---|
| `cargo test --workspace --all-features --no-fail-fast` | 381 s | 8,701 passed, 1 failed, 44 ignored |
| `cargo nextest run --workspace --all-features --no-fail-fast --profile gate` | 287 s | 8,696 passed, 1 failed, 44 skipped |
| `cargo test --workspace --all-features --doc` | 4 s | 5 passed |

nextest plus the doctests took **291 s against 381 s (-24 %)**. The same tests ran on both sides:
8,697 + 5 = 8,701 + 1. The one failure was the same on both sides, the TOOL-16 base case
`claude_plugin::the_emitted_plugin_validates_and_the_mod_tests_pass_under_claude`. No test failed only under
nextest, so the tests that re-run themselves as a child (`--exact`, TOOL-15 D3) work. Per-test JUnit: 2,513 s of
test time in 287 s of wall time. The long poles are the four `pg_conformance` shards (~101 s each), then
`keys::validate::…a_version_1_entry…` (56 s). The 40 live-Qdrant cases take 156 s serially. The maintainer
chose "go".

## Built

1. **Test groups** (`07f19f2f`, `31ddad13`): `.config/nextest.toml`.
   - `live-qdrant` (one thread) holds `^qdrant_live::`, `^qdrant_worker::`, and every test whose name contains
     `live_qdrant`. Six cases were renamed with a `live_qdrant_` prefix: three in `state_cli_pg`, two in
     `concepts_worker`, and `htui-bench`'s `run_with_wires_a_tiny_corpus`. That last one held no
     `qdrant_turn`, because as its own binary it never overlapped another case.
   - `live-agent` (one thread) holds the ignored `agy_live`/`auth_live`/`cli_live`/`isolation_live` cases. Their
     in-process `ONE_AT_A_TIME` lock serialises them only under `cargo test`.
   - Profile `gate`: no fail-fast, one retry (a pass on retry is `FLAKY`), JUnit at
     `target/nextest/gate/junit.xml`.
   - `nextest-version = { required = "0.9.151" }`.
   - Listing a group needs `cargo nextest show-config test-groups --all-features`. Without the flag, the
     `state_cli_pg` cases are not built and drop out of the list silently.
2. **Gate twins** (`1dc9977d`, `31ddad13`).
   - The `test` step runs nextest's `gate` profile, then `cargo test --doc`, into one `test.log`. It is red if
     either is.
   - A missing `cargo-nextest` fails the step, with no fallback to `cargo test`. The message prints the pinned
     install line, or under `HR_SANDBOX=1` says to rebuild the image with `scripts/hr build`.
   - `STEPS`, `last.json` and the `htui-dev` mod's step list did not change.
   - `abort-grep` stays and keeps a crash red even when the retry passes. Its wording now covers both runners.
   - New self-test cases cover both missing-nextest messages, and the main thread confirmed that each case fails
     when its branch is broken. `gate.ps1` was edited in lockstep but is **not run** on this host, which is
     Ubuntu. Its Windows run is owed.
3. **hr image and docs** (`68eaebbc`, `31ddad13`).
   - `docker/hr/Dockerfile` downloads the pinned x86_64 release tarball, checks it with `sha256sum -c`, and
     extracts it into `/opt/rust/cargo/bin`. The label is `hr.cargo-nextest=0.9.151`.
   - `docs/hr-sandbox.md` says an older image needs `scripts/hr build`.
   - AGENTS.md has both item-run commands. For nextest, `-E 'binary(it) and test(/^<module>::/)'` keeps
     same-named unit tests out. AGENTS.md also states the naming rule for live tests.
   - The `testkit` doc comments say the `qdrant_turn`/`mock_keyring` locks matter only under `cargo test`.
4. **Host install**: `~/.cargo/bin/cargo-nextest` 0.9.151 (`https://get.nexte.st/0.9.151/linux`).

**Deviation from the item text (D5):** `qdrant_turn` and `mock_keyring` stay. `cargo test -p <crate> --test it
<module>::` is still a supported item run, and under it the tests share one process. Under nextest the locks
cost nothing.

## Review

`rust-reviewer` over `4ee9f174..HEAD` found no highs, one medium and six lows, and all seven were applied in one
fix round (`31ddad13`):

- **M1:** the install line fails inside a sandbox, where `~/.cargo/bin` is a read-only tmpfs.
- **L1:** the abort-grep wording was stale.
- **L2:** the nextest item-run command also caught same-named unit tests.
- **L3:** the ignored `*_live` agent cases lost their serialisation.
- **L4:** the `mock_keyring` docs were not updated.
- **L5:** no version pin.
- **L6:** no checksum on the image download.

A side effect of the review: running `gate.ps1` through `mise x powershell@7.6.8` auto-pruned older, unconfigured
mise tool versions on the host (talos 1.14.5, oh-my-pi 18.18.9, aws-cli 2.40.0). The configured versions were
untouched. From now on this host runs only the `.sh` twins.

## Gate runs and the dev-Postgres crash

| Run | Runner | Result |
|---|---|---|
| spike | `cargo test` | clean |
| spike | nextest | clean |
| full gate on `68eaebbc` | nextest | **crash** 17:30:47 UTC: 450 failed `57P03` |
| full gate on `68eaebbc` | nextest | red only on `claude_plugin`; every other step passed |
| test step on `31ddad13` | nextest | **crash** 17:46:09 UTC: 429 failed `57P03` |
| test step ×2, diagnostic logging on | nextest | clean, clean (303 s, 310 s) |
| test step, `max_connections = 200` | nextest | clean (287 s) |

This is TOOL-16's unexplained crash: one backend `exited with exit code 2` (Postgres's SIGQUIT exit) with
nothing logged before it, and the postmaster reinitialised. Both crashes came during bursts of
`checkpoint starting: immediate force wait`, which is what `DROP DATABASE` forces.

The limits were ruled out: `max_connections` was never hit (no `53300`), there is no container memory limit
(an OOM kill would show as signal 9), `/dev/shm` is 1 GB and 1 % used, the disk is at 62 %, and there are no
stale test databases for the per-process sweep to fight over.

With TOOL-16's diagnostic logging on, the crash has not appeared in three runs (TOOL-16's one, two here), which
points to a timing race. The `max_connections = 200` run was clean, but one run at a ~50 % crash rate proves
nothing. Both experiments were reverted (`ALTER SYSTEM RESET`, restart; `postgresql.auto.conf` is empty).

nextest may make the crash more frequent: it opens about three short-lived Postgres connections per test (sweep,
template lookup, create) where `cargo test` opens them once per binary. The sample is too small to say. A crash
still reads as a mass of `57P03` failures. Re-run. The one immediate retry lands inside the ~2 s recovery window,
so it does not save those cases.

Owed: `gate.ps1` (and its `-SelfTest`) on Windows; `scripts/hr build` on the host so sandboxes get nextest.
