# MOD-79 - MCP token off claude's argv (done, 2026-10-06)

**Requirements:** `R-MCP-1`, `R-NF-1`.
**Origin:** MOD-11 (`docs/decisions/mod/mod-11.md`), blueprint E-1 and review L2.
**Artifacts:**
- plan [`.claude/plans/mod-79-mcp-token-off-argv.plan.md`](../../../.claude/plans/mod-79-mcp-token-off-argv.plan.md):
  requirements 1-6, D1-D3, the Windows options table, verified-claims table;
- blueprint `.claude/plans/mod-79-mcp-token-off-argv.blueprint.md`: T1-T5 designs, the 13 `argv` call sites, the
  hazards an implementer must not trip on.

Decision numbers are local to MOD-79 (the MOD-31 convention).

Routed as **plan** (0 criteria fired). Run in a TOOL-7 sandbox (`hr/MOD-79`). T1 → T2 → T3 ran serially in one lane;
T4 and T5 ran in parallel lanes with disjoint file sets.

## The problem

`claude-cli` got the per-session `HTUI_MCP_TOKEN` inline in `--mcp-config=<json>`, so any local account could read
it from `/proc/<pid>/cmdline`. The token was inert without the `0700` socket directory (MOD-11 E-1), but it did not
belong on a command line. Review L2 had been refuted as a live leak (no `Debug` path logged it), so the second half is
hardening.

## What was built

- **The config travels as a file.** `cli::McpConfigFile` writes `cli::mcp_config(servers)` (the same JSON as before)
  to `mcp.json` with `create_new` and mode `0600`, inside a per-session `0700` directory `htui-cli-<pid>-<8 hex>`.
  `cli::argv` takes `mcp_config: Option<&Path>` and pushes one `=`-joined `--mcp-config=<path>` in the same position
  as before (MOD-11 D8 ordering unchanged). The write runs on a blocking thread (`contained::spawn_blocking`); its
  errors read `agent spawn failed: cannot write the MCP config: …` and name the base or file they tried.
- **The session owns the file.** `open_session` takes the guard and moves it into the task's future, after
  `run_session`, so it is dropped after the final kill on a normal end and with the future on the timeout abort.
  A failed spawn drops it in `io()`. A parameter, not a `SessionOptions` field: the field would need an `Arc` to keep
  `Clone`, and a clone would outlive the session.
- **One private-directory helper.** `htui_agent::private_dir` (moved from `htui-mcp/src/channel.rs`, which now calls
  it with `htui-mcp`): base `$XDG_RUNTIME_DIR` when absolute and a directory, else the temp dir; mode `0700` at
  `mkdir`; the check uses `symlink_metadata`, requires a directory and refuses group/other bits. Same base as the
  socket, so isolation that lets the relay reach the socket lets the CLI read its config (Unix).
- **Windows (option A, maintainer 2026-10-06).** The directory goes under `std::env::temp_dir()`
  (`%LOCALAPPDATA%\Temp`) and inherits the profile's DACL: owner, SYSTEM, Administrators. `unsafe_code = "forbid"`
  rules out setting a DACL in-tree; `icacls` (B) and a DACL crate (C) were rejected. Compiled by the
  `x86_64-pc-windows-gnu` cross-check; runtime verification belongs to MOD-16.
- **`ResolvedLaunch`'s `Debug` redacts `--mcp-config`.** `RedactedArgs` prints `--mcp-config=[REDACTED]` for the
  joined form and `[REDACTED]` for each value of the split form up to the next `-`-prefixed argument. `driver.rs`'s
  `REDACTED` became `pub(crate)` so both wrappers print the same marker.
- **Docs.** `docs/htui-mcp.md`: "The token is never on a command line", the `htui-cli-*` directories in the crash
  note and the cleanup `ls`, and a troubleshooting entry for the write error.

Rejected: passing the token through claude's environment with `${VAR}` expansion. The agent's `Bash` children inherit
it, so one `env` in a turn would record the token in the transcript.

## Decisions (maintainer, 2026-10-06)

- route accepted (plan path), no ultracode;
- plan confirmed, with Windows option A;
- review (rust-reviewer: approve, LOW only): L1, L2, L3, L5, L6, L7 applied; L4 skipped (`argv` stays a pure
  function over `Option<&Path>`; its 13 test call sites pass plain paths); L8 closed by this bookkeeping and a `/tmp`
  cleanup.

## Plan claims corrected by the blueprint

`argv` has 13 test call sites, not 8 (five multi-line calls were missed); `RedactedEnv` lives in `driver.rs`, so T4
also touched it; `htui-agent` needed `uuid`'s `v4` feature for `cargo check -p htui-agent` on its own (it compiled
only through `htui-mcp`'s feature unification). The plan's verified-claims table records all three.

## Commits

- plan, blueprint: `472a1b6d`, `4e0ebe0e`;
- T5 docs: `be9a6a30`; T4 `RedactedArgs`: `3715673a`;
- T1 `private_dir`: `281741f2`; T2 `McpConfigFile`: `9a3fcc45`; T3 argv path and session-owned guard: `225a5aa2`;
- review L2, L6: `1d0ff73d`; L1: `6a1ea625`; L3, L5, L7: `050e8873`.

## Verification

- `cargo test -p htui-agent --features test-support --no-fail-fast -- --test-threads=1`: 614 passed;
  `cargo test -p htui-mcp --all-features`: 90 passed; `cargo test -p htui --all-features -- mcp`: 12 passed.
- `cargo clippy --workspace` with and without `--all-targets --all-features`, `-D warnings`: clean; `cargo fmt`: clean.
- `CC=gcc AR=ar cargo check -p htui-agent -p htui-mcp --target x86_64-pc-windows-gnu`: clean (library code; the
  Windows test cfgs need a mingw C compiler for `aws-lc-sys` and were not compiled).
- New process tests in `tests/cli_driver.rs`: the planted token is in no argv element, the fake CLI can `cat` the
  config, and the directory is gone after a cancel, after an exit before `system/init` and after an init timeout.
  A unit test proves a failed spawn leaves nothing behind (checked by forcing a leak with `mem::forget`).
- Live: claude 2.1.289 started the stdio server from `--mcp-config=<path>` to a `0600` file (planning probe). No
  end-to-end live session was run: the live cases spend model tokens and none covers MCP.

## Carried

- An operator's own inline `--mcp-config` JSON in `CliSettings.extra_args` or `AgentLaunch.args` still prints in
  those types' `Debug` (reviewer note, outside this item's scope). Not filed; the maintainer decides whether it
  becomes an item.
- Windows runtime behaviour of the inherited DACL: MOD-16.
