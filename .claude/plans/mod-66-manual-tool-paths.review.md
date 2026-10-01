# MOD-66 review: `rust-reviewer`, 2026-10-01

**Scope:** `git diff 4b9efa0..HEAD -- crates/` at `hr/MOD-66` HEAD, after the wave-2 merge (11 commits).
**Verdict:** approve with fixes. No CRITICAL or HIGH findings.
**Maintainer disposition, 2026-10-01:** fix all findings on this branch, M1–M2, L1–L4 and N1–N5.

The reviewer found no behavioural defect in the checks against the spec:
- tier order with no fall-through;
- `source` and the carried map;
- B1 "always writes";
- the claim RAII on success, panic and abort;
- three-way routing;
- `busy` clearing, H-12, B17 and B18;
- R-SEC-2.

It recorded one accepted behaviour change (D6): existing hand-written ANA-4 manual rows now spawn their recorded `resolved` instead of going through `launch::resolve`.

## MEDIUM

**M1. The claim-held-for-the-task's-lifetime rule (H-10) is not pinned.**
- **Where:** `crates/htui/src/agent_worker.rs` (~:11115 `a_valid_map_writes_a_manual_snapshot_and_answers_tool_paths`, ~:11070).
- **Problem:** dropping the claim on the request arm before `tokio::spawn` would reopen the D60 race, and every test would stay green.
- **Fix:** mirror `a_flow_holds_the_reprobe_claim_for_its_row` (~:8576). A manual path to a holding fixture script with `handshake: true` keeps the task in flight. Assert `reprobe_claims.claim((agent_id, ids::BOX)).is_none()` while it is in flight, and `is_some()` after `finish_background`.

**M2. No test checks that the real handler uses the `Writes::ToolPaths` tag.**
- **Where:** `agent_worker.rs` (~:1470; the tests ~:11089 and ~:10994 push a hand-built `Background::writing_tool_paths(pending())`).
- **Problem:** if `set_tool_paths` pushed `Background::writing(..)`, everything would stay green, and `ProbeAgents` would run beside a real write (H-11).
- **Fix:** with M1's in-flight setup, send `ProbeAgents` while a real `SetToolPaths` is in flight and assert the refusal is `tool_paths_running(agent_id)`. A second `SetToolPaths` must get the B5 sentence.

## LOW

**L1. `is_file` runs on the store loop's request handler** (`agent_worker.rs` ~:1460).
- **Problem:** a hung NFS or autofs path stalls every tab's store requests.
- **Fix:** move the `is_file` loop into `run_tool_paths`, before the probe, with the claim already held, and answer `Failed` from there.

**L2. `clearing_every_field_sends_an_empty_map` is not portable to macOS** (`crates/htui/tests/settings.rs` ~:5192).
- **Problem:** 64 backspaces leave `"/"` when `$TMPDIR` is long.
- **Fix:** backspace `alpha.chars().count()` times.

**L3. Nothing pins the worker's launch-parse refusal, which carries an R-SEC-2 promise** (`agent_worker.rs` ~:1438).
- **Fix:** a worker test whose malformed `launch` holds `"env": {"K": "SECRET-SENTINEL"}`. Assert the exact sentence and `!message.contains("SECRET-SENTINEL")`.

**L4. The generic background sentence says "install once it has finished"** (`agent_worker.rs` ~:1909).
- **Problem:** a tool-paths user refused during `r` or a staleness re-probe is told to install.
- **Fix:** make the sentence verb-neutral ("…; try again once it has finished") and update the existing pins.

## NIT

- **N1.** `agents.rs` ~:1282, `lists_a_manual_row`: count only manual rows whose cell can show `*`. That means `!summary.user_off`, and no note while `self.probing`.
- **N2.** The routing tests (`store_worker.rs` ~:4014 and the `testkit.rs` harness test) also assert `message.contains("not found")` now that T3 has landed.
- **N3.** `PathsForm`'s derived `Debug` prints `opened` (the paths). Write it by hand to print `opened.len()`, consistent with `PathField`.
- **N4.** The worker refuses `SetToolPaths` on a row with no `discovery.tools`, using the UI's `LITERAL_LAUNCH` sentence. Share the const.
- **N5.** `probe.rs` ~:1418: move the map out of `stored` instead of `.clone()`. Use `AgentLaunch::deserialize(&agent.launch)` instead of `from_value(agent.launch.clone())` at `agent_worker.rs` ~:1437 and `agents.rs` `PathsForm::open`.

## Fixes

To be filled in at close-out: the commit hashes per finding.
