# Plan: MOD-79 - MCP token off claude's argv

**Source**: `HANDOFF.md` MOD-79 (from MOD-11, `docs/decisions/mod/mod-11.md`, blueprint E-1, review L2)
**Requirements**: `R-MCP-1`, `R-NF-1`
**Routed**: plan path (0 criteria fired), accepted by the maintainer 2026-10-06
**Complexity**: Small
**Status**: done 2026-10-06 (`docs/decisions/mod/mod-79.md`; Windows ACL: option A)

## Summary

`claude-cli` is started with `--mcp-config=<json>`. The JSON's `env` carries the per-session
`HTUI_MCP_TOKEN`, so any local account can read the token from `/proc/<pid>/cmdline`. The token is
inert without the `0700` socket directory, but it does not belong there. After this change the CLI
driver writes the same JSON into a `0600` file in a per-session `0700` directory and passes
`--mcp-config=<path>`. The file lives exactly as long as the session task. `ResolvedLaunch`'s
`Debug` also redacts every `--mcp-config` value, so neither a path nor an operator's inline JSON
reaches a log line.

## Requirements restated

1. No `HTUI_MCP_TOKEN` value (or any `McpServerSpec.env` value) appears in the CLI child's argv.
2. The config travels as a file: `0600` (Unix) inside a private per-session directory (`0700`,
   checked like `channel.rs` checks the socket directory). On Windows it goes under the per-user
   temp dir. See the Windows decision below.
3. The file and its directory are removed when the session ends: normal end, cancel, the
   `open_session` failure and timeout arms (an aborted task drops its state), and a failed spawn.
4. `ResolvedLaunch`'s `Debug` prints `--mcp-config=[REDACTED]` for the `=`-joined form. For the
   split form `--mcp-config a b` it prints `[REDACTED]` for every following argument up to the next
   `-`-prefixed one. Every other argument prints unchanged.
5. The ACP path is unchanged: the token reaches an ACP agent over stdin (`session/new`), never argv.
6. Argv order and shape (MOD-11 D8, D18, MOD-26 D11) are unchanged except for the config value
   itself: still one `=`-joined argument, after the pairs, before the narrowing and `extra_args`.

## Design choices (recommendations; CONFIRM covers them)

- **D1: the driver writes the file, not `McpHost::open`.** Blueprint E-1 sketched the file in the
  socket directory, written by `McpHost::open`. The driver side is better on three counts. The JSON
  shape is the CLI's dialect (`cli::mcp_config`), which `htui-mcp` should not emit. It covers every
  `McpServerSpec` a session carries, including fakes. It also needs no new field on
  `McpServerSpec`/`ToolLease`, and no Windows special case for a pipe that has no directory. The
  cost is a second private directory per CLI session that has MCP servers.
- **D2: one private-directory helper.** `channel.rs`'s `base()` + `private_dir()` move into
  `htui-agent` (on which `htui-mcp` already depends) as one helper with a name prefix parameter.
  `channel.rs` calls it with `htui-mcp`, the driver with `htui-cli`. Same base
  (`$XDG_RUNTIME_DIR`, else the temp dir), so whatever isolation lets the relay child reach the
  socket also lets the CLI read its config.
- **D3: the file outlives `system/init`.** The CLI reads it at start-up, but the guard is held for
  the whole session rather than dropped after init: the CLI's own reconnects are not ours to
  predict, and a session-scoped guard is the invariant a reader can check.
- **Rejected: pass the token via env and `${VAR}` expansion.** That avoids a file, but the agent's
  `Bash` children inherit claude's environment, so a single `env` in a turn would print the token
  into the recorded transcript.

### Windows ACL: option A chosen at CONFIRM (2026-10-06)

The workspace sets `unsafe_code = "forbid"` (`Cargo.toml:182`, every crate `[lints] workspace =
true`). `forbid` cannot be lowered by an inner `allow`, so the `windows` crate's
`SetNamedSecurityInfoW`/`InitializeAcl` path is out of reach in-tree. The options:

| Option | What | Cost |
|---|---|---|
| **A (recommended)** | Directory under `std::env::temp_dir()` (`%LOCALAPPDATA%\Temp`), which inherits the profile's DACL: owner, SYSTEM, Administrators. Documented as the Windows equivalent, as MOD-11 documented the pipe's default DACL. | No `unsafe`, no dependency. No runtime check of the DACL; runtime verification is MOD-16's. |
| B | `icacls <dir> /inheritance:r /grant:r <user>:(OI)(CI)F` subprocess after creating the dir | An explicit protected DACL, but a process spawn per session start, and user-name or SID resolution. |
| C | New dependency with a safe DACL API | An explicit DACL; a new crate for one call site, Windows-only, checkable only by cross-compiling. |

On Windows the command-line exposure is also narrower: reading another user's process command line
needs `PROCESS_VM_READ` on that process.

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Private dir | `crates/htui-mcp/src/channel.rs:488-510` | `DirBuilder::mode(0o700)`, then re-read the mode and refuse `& 0o077 != 0`, removing the dir |
| RAII cleanup | `crates/htui-mcp/src/channel.rs` `Listener::close`/`Drop` | Best-effort remove file then dir; idempotent |
| Debug redaction | `crates/htui-agent/src/launch.rs:452-461` (`RedactedEnv`) | Hand-written `Debug` wrapping the sensitive field |
| Errors | `crates/htui-agent/src/cli/mod.rs` `launch_for` | `DriverError::Spawn`/`Transport` with a sentence naming what failed |
| Pure argv tests | `crates/htui-agent/tests/cli_driver.rs:466-600` | `argv(...)` asserted as a list; `mcp_configs` helper |
| Process tests | `crates/htui-agent/tests/cli_driver.rs:868+` | `#!/bin/sh` fake CLI scripts in a tempdir |
| Debug tests | `crates/htui-agent/tests/launch.rs:419` | `debug_never_prints_an_environment_value`: plant a value, assert absent |

## Files to change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-agent/src/private_dir.rs` | CREATE | shared `0700` per-session dir helper (D2) | T1 |
| `crates/htui-agent/src/lib.rs` | UPDATE | `pub mod private_dir;` | T1 |
| `crates/htui-mcp/src/channel.rs` | UPDATE | use the helper; delete its private copy | T1 |
| `crates/htui-agent/src/cli/mcp_file.rs` | CREATE | `McpConfigFile`: write `0600` `mcp.json`, `path()`, `Drop` removes | T2 |
| `crates/htui-agent/src/cli/mod.rs` | UPDATE | `mod mcp_file;`; `argv` takes `mcp_config: Option<&Path>`; `io()` writes the file; the guard reaches `run_session`; docs 6¼ | T2, T3 |
| `crates/htui-agent/tests/cli_driver.rs` | UPDATE | 8 `argv` call sites; path-form assertions; process test: no token in argv, file lifetime | T3 |
| `crates/htui-agent/src/launch.rs` | UPDATE | `ResolvedLaunch` `Debug` redacts `--mcp-config` values | T4 |
| `crates/htui-agent/tests/launch.rs` | UPDATE | redaction tests (both forms, neighbours intact) | T4 |
| `docs/htui-mcp.md` | UPDATE | rewrite the "token is visible in the command line" bullet (`:187-191`) | T5 |

## Tasks (TDD: tests first in each)

### T1: shared private-directory helper
- **Action**: `htui_agent::private_dir::create(prefix) -> io::Result<PathBuf>`. On Unix: base
  `$XDG_RUNTIME_DIR` (absolute and a dir) else `temp_dir()`, name `<prefix>-<pid>-<8 hex>`, mode
  `0700`, refuse and remove if `mode & 0o077 != 0`. On Windows: the same name under `temp_dir()`,
  plain `create_dir` (option A). Switch `channel.rs` `platform::private_dir`/`base` to it.
- **Mirror**: `channel.rs:488-510`, moved verbatim.
- **Validate**: `cargo test -p htui-agent --features test-support private_dir`; `cargo test -p htui-mcp --all-features channel`.

### T2: `McpConfigFile`
- **Action**: `McpConfigFile::write(servers: &[McpServerSpec]) -> io::Result<Option<Self>>`. It
  returns `None` for an empty slice. Otherwise it creates a dir via T1 (`htui-cli`), then writes
  `cli::mcp_config(servers)` to `<dir>/mcp.json` with `create_new(true)` and `.mode(0o600)` on Unix.
  `path()` returns the file. `Drop` removes the file, then the dir, best effort. `Debug` prints the
  path only.
- **Tests**: content equals `mcp_config(servers)`; file `0600` and dir `0700` (Unix); both gone
  after drop; `None` for no servers.
- **Depends on**: T1.

### T3: argv carries the path; the guard lives with the session
- **Action**: `argv(row_args, cli, spec, session_id, mcp_config: Option<&Path>)` pushes
  `--mcp-config=<path>` when `Some`. It no longer calls `mcp_config` itself; that function stays
  `pub` and tested. `CliDriver::io` writes the file before `spawn` and maps an I/O error to
  `DriverError::Spawn("cannot write the MCP config: …")`. The guard then travels to `run_session`
  so that every exit drops it, including the abort in `open_session`'s timeout arm. The mechanism
  is the architect's call: a `SessionOptions` field (`Option<Arc<McpConfigFile>>`, keeps `Clone`)
  or an `open_session` parameter. A failed spawn drops it in `io()`.
- **Tests**: update the 8 `argv` call sites. `argv_with_mcp_has_one_joined_config_before_tools`
  keeps its positional checks and passes a path. New process test: a fake CLI script records its
  argv and `cat`s the config path to a side file. Assert the planted token appears in no argv
  element, that the side file holds the token (the CLI could read it), and that the dir is gone
  after the session ends. Also assert the dir is gone after an `open_session` failure (script exits
  before `system/init`).
- **Depends on**: T2.

### T4: `ResolvedLaunch` `Debug` redaction
- **Action**: render `args` through a `RedactedArgs` wrapper (redaction rule: requirement 4).
- **Tests**: `--mcp-config={"…token…"}` is redacted, as is the split form with two values, and
  `--model x` stays as it is.
- **Independent**: file set `{launch.rs, tests/launch.rs}`, disjoint from T1–T3 and T5.

### T5: docs
- **Action**: `docs/htui-mcp.md:187-191`: the token reaches `claude-cli` through a `0600` file in a
  private directory, with argv carrying only the path. Windows wording per the CONFIRM decision.
- **Independent**: file set `{docs/htui-mcp.md}`.

**Lanes**: T1 → T2 → T3 run serially (one implementer: they share `cli/mod.rs` and each builds on
the last). T4 and T5 run in parallel with that chain.

## Validation

```bash
cargo fmt --all --check
cargo test -p htui-agent --features test-support --no-fail-fast -- --test-threads=1
cargo test -p htui-mcp --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
# Windows cfg paths (target must be added in the sandbox first; CC/AR per memory note)
rustup target add x86_64-pc-windows-gnu
CC=gcc AR=ar cargo check -p htui-mcp -p htui-agent --target x86_64-pc-windows-gnu
```

Plus one live probe on the installed CLI: a session's argv (`/proc/<pid>/cmdline`) holds a path and
no token.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The guard dropped before the CLI reads the file (e.g. owned by `io()`'s frame) | Medium | T3's process test reads the file *through the fake CLI*; the guard is owned by the session task |
| Leaked files on `htui` crash | Low | The token dies with the process (in-memory registry); the dir is `0700`; same leak class as the socket dir |
| An isolate that hides the base dir from the agent | Low | Same base as the socket the relay already reaches (D2) |
| Windows behaviour unverified at runtime | Medium | Cross-check compiles `cfg(windows)`; runtime is MOD-16's |
| `cli_driver.rs` churn conflicts with sibling runs (MOD-78 touches the MCP session) | Low | MOD-78 lives in `htui-mcp` session/host; this plan touches `channel.rs` only in T1 |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `claude` accepts `--mcp-config=<path>` (`=`-joined, one argv element) and spawns the file's servers with the file's `env` | ✓ verified | Probe, claude 2.1.289: `--mcp-config=/tmp/mcpprobe/c.json` (mode 0600), and the stdio server logged `T=from-file` |
| The CLI help lists `--mcp-config <configs...>` as "JSON files or strings" | ✓ | `claude --help` line 132 |
| The token is put in argv today by `cli::argv` via `mcp_config(&spec.mcp)` | ✓ | `crates/htui-agent/src/cli/mod.rs:182-184`, `:213-232`; token minted into `env` at `crates/htui-mcp/src/host.rs:414-422` |
| `ResolvedLaunch`'s `Debug` prints `args` unredacted (L2) | ✓ (`RedactedEnv` itself lives in `driver.rs:325`, so T4 also touches `driver.rs`) | `crates/htui-agent/src/launch.rs:452-461` |
| No current `Debug` path logs the token (L2 refuted) | ✓ (recorded) | `docs/decisions/mod/mod-11.md:244`; the change is hardening |
| `unsafe_code = "forbid"` workspace-wide; `htui-agent` inherits it | ✓ | `Cargo.toml:182`; `crates/htui-agent/Cargo.toml:83-84` |
| `htui-mcp` depends on `htui-agent` (it can host the shared helper) | ✓ | `crates/htui-mcp/Cargo.toml:17`; `channel.rs` calls `htui_agent::contained::spawn` |
| The private-dir pattern exists only in `channel.rs` | ✓ | text search: `private_dir` only at `channel.rs:497,518` |
| `argv` has 1 production and 8 test call sites | ✗ corrected by the blueprint: 13 test call sites | `cli/mod.rs:453`; `tests/cli_driver.rs:171,207,249,472,473,543,582,590` |
| `SessionOptions` is built literally at 2 sites | ✓ | `cli/mod.rs:501`; `tests/cli_driver.rs:1447` |
| `open_session`'s timeout arm aborts the task, so task-owned state is dropped | ✓ | `cli/mod.rs:842-853` |
| The ACP path never puts the spec on argv | ✓ | `docs/htui-mcp.md:190-191`; `tests/acp_driver.rs:2008` (`session_new_carries_the_mcp_server_as_stdio`) |
| `tempfile` is a dev-dependency only (the product code must not need it) | ✓ | `crates/htui-agent/Cargo.toml:78` under `[dev-dependencies]` |
| The Windows GNU target is not installed in this sandbox | ✓ | `rustup target list --installed` → linux only; validation adds it |
| Task independence: T4 `{launch.rs, tests/launch.rs}` ∩ T5 `{docs/htui-mcp.md}` ∩ T1–T3 `{private_dir.rs, lib.rs, channel.rs, cli/mcp_file.rs, cli/mod.rs, tests/cli_driver.rs}` = ∅ | ✓ | File table above; T1–T3 share `cli/mod.rs`, hence serial |
| `htui-agent` builds alone with `Uuid::new_v4` | ✗ found by the blueprint: needs `uuid` feature `v4` (it compiled only through `htui-mcp`); added in T1 | `crates/htui-agent/Cargo.toml` |

## Acceptance

- [ ] No `McpServerSpec.env` value in the CLI child's argv (process test plus live probe)
- [ ] Config file `0600` in a `0700` per-session dir; removed on every session exit
- [ ] `ResolvedLaunch` `Debug` redacts both `--mcp-config` forms
- [ ] `docs/htui-mcp.md` updated; Windows equivalent documented per the CONFIRM decision
- [ ] Validation passes, including the Windows cross-check
