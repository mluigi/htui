# TOOL-17 fix round (verify: fix-round)

Reviewer findings on `4ee9f174..HEAD`. **Confirm each against the code before acting.** If one turns out
wrong, skip it and say why. Plan: `.claude/plans/tool-17-cargo-nextest.plan.md` (decisions D1–D7).

## M1 — the gate's install line fails inside an hr sandbox
`scripts/gate.sh` (install line near the top, used by `step_test`) and `scripts/gate.ps1` (lines ~40-44).
In a sandbox `~/.cargo/bin` is a read-only empty tmpfs that is not on PATH (`docker/hr/compose.hr.yaml:94,178`).
**Fix:** when `HR_SANDBOX=1`, the missing-nextest message says the image predates TOOL-17 and must be rebuilt on
the host with `scripts/hr build`, and prints no install line. Otherwise the install line extracts into
`${CARGO_HOME:-$HOME/.cargo}/bin`. Keep the twins in lockstep. Extend the existing missing-nextest self-test case
in both twins with an `HR_SANDBOX=1` variant that asserts the rebuild message.

## L1 — abort-grep wording is stale under nextest
`gate.sh:18-19` and the `step_abort_grep` message, and the same places in `gate.ps1`. Under nextest a crashed test
is counted as a failure, and it can pass on its one retry (`FLAKY`) while the SIGABRT text stays in the log.
**Fix:** reword the header and the message along these lines: "a test aborted. Under nextest it may have passed on
retry; under cargo test the rest of its binary did not run." abort-grep keeps a crash red even when the retry
passes. That is deliberate, so say so in the header comment.

## L2 — the nextest item-run command in AGENTS.md also runs same-named unit tests
`AGENTS.md` (~53-55). **Fix:** `cargo nextest run -p <crate> --all-features -E 'binary(it) and test(/^<module>::/)'`.

## L3 — ignored `*_live` agent tests lose their in-process one-at-a-time lock under nextest
The `ONE_AT_A_TIME` locks are at `crates/htui-agent/tests/it/{agy_live,auth_live,cli_live,isolation_live}.rs`
(check which `*_live` modules exist and which hold such a lock). **Fix:** in `.config/nextest.toml`, add a test group
`live-agent = { max-threads = 1 }` with an override whose filter matches exactly the `*_live::` modules that
serialise today, e.g. `binary(it) and test(/^(agy|auth|cli|isolation)_live::/)`. Verify with
`cargo nextest show-config test-groups --all-features --run-ignored all` that it lists those cases and nothing
else. Then add one line to AGENTS.md next to the live-Qdrant rule.

## L4 — D5 asks for both runners in the `mock_keyring` docs too
`crates/htui-store/src/testkit.rs` `KEYRING` / `mock_keyring` doc comments (~516-540). **Fix:** one sentence:
under nextest each test process has its own fake slot, so the lock only matters under `cargo test`. Doc
comments only.

## L5 — the pinned nextest version is never checked
**Fix:** at the top of `.config/nextest.toml`, add `nextest-version = { required = "0.9.151" }` (minimum version).
nextest then refuses an older binary with a clear message. Check it with `cargo nextest show-config test-groups --all-features`.

## L6 — Dockerfile download has no checksum
`docker/hr/Dockerfile` (the nextest `RUN`). **Fix:** download the release tarball by its full URL,
`https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-0.9.151/cargo-nextest-0.9.151-x86_64-unknown-linux-gnu.tar.gz`,
to a temp file. Verify it with
`echo "1d4ce302b820c41f4b89c5fb9bce45f0f0c94882c1cf08fc360caf08be6e7df4  <file>" | sha256sum -c -`, extract it
into `/opt/rust/cargo/bin`, and remove the file. The main thread verified the sha256 from two independent
downloads. The host is x86_64 only, so add no arm64 branch, but say x86_64 in the comment. Do not run `docker build`.

## Checks (yours, crate-scoped; never the gate, never workspace tests)
`bash scripts/gate.sh --self-test`. This host is Ubuntu: edit `gate.ps1` in lockstep but **do not run it**,
no `pwsh`, no `mise x`/`mise install` (it auto-prunes the maintainer's tools). Say in your report that the ps1
twin was edited but not run. Also run
`cargo nextest show-config test-groups --all-features --run-ignored all`, `cargo fmt --check`, and
`bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.
