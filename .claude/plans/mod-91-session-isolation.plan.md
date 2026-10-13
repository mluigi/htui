# Plan: MOD-91 — keep the user's mods, settings hooks and skills out of htui-driven sessions

**Status: DONE 2026-10-13 — CONFIRMED 2026-10-08 (D1–D7; D8 added after T1); implemented T1–T6, review R1 applied; write-up `docs/decisions/mod/mod-91.md`.**

**Source**: `HANDOFF.md` MOD-91 (from ANA-29, `docs/ANA-29.md` §4.4, §5.5, §6.1 item 1, §11).
**Routing**: the decision table gave the PRD path (C3 and C4 fired, at threshold, low confidence).
**Routed as plan by maintainer override (2026-10-08).**
**Requirements**: `R-ID-5` (system skills disabled), `R-AGT-1`, `R-AGT-4`, `R-TUI-6`, `R-HIS-1`
(the relay and its record stay the only approval path).
**Complexity**: Medium. A probe gates everything. There is no migration and no new `StoreRequest`.
**Staffing**: every agent inherits the session model, with no `model:` override. The reviewer is
`rust-reviewer` (`.claude/workflow-config.json`).

## Summary

htui starts Claude Code in two ways. One is `claude -p` (the `claude-cli` row, `cli::argv`). The
other is `claude-agent-acp` (the `claude` row, ACP). Today neither isolates the session, so the
user's plugins, settings hooks, skills and **mods** load into every session htui drives. A mod
can then approve a tool call before `mcp__htui__permission_prompt` is ever asked. A live probe
decides which levers work without breaking subscription auth or htui's own MCP server. Those
levers then become the default for both transports, with a per-agent-row opt-back-in in
Settings > Agents. `HTUI_SESSION=1` is set in every agent's environment, so a mod that is let
back in can stand down.

## Decisions (defaults the plan assumes; maintainer may override at CONFIRM)

- **D1 — CLI levers.** Use `--settings '{"disableAllHooks":true}'` and `--disable-slash-commands`
  (ANA-29 §5.5), unless the probe falsifies them. In that case T1's results amend D1 before T3
  starts, and the maintainer re-confirms only if the lever *set* changes. `--safe-mode` and
  `--setting-sources` are probed for the record only.
- **D2 — Position on the argv.** Emit the levers after the narrowing (6½/6¾) and **before**
  `extra_args`, so the operator's `extra_args` still win (the `argv` doc's rule 7). Emit them on
  **every** session, tool-less help turns included.
- **D3 — The opt-in.** Add a new top-level `agent.settings.user_extensions: bool` (`AgentSettings`,
  `#[serde(default)]` → `false` = isolated). It applies to both transports, so it sits beside
  `permission`, `quota` and `usage`, not under `cli`. No migration is needed: the column is
  `JSONB NOT NULL DEFAULT '{}'`, and every existing row reads `false`. A settings document that
  fails to parse already falls back to `AgentSettings::default()`
  (`cli/mod.rs:471`, `acp/mod.rs:270`), which is the isolated side. That is the safe direction.
- **D4 — `HTUI_SESSION=1`.** Set it on the resolved launch env in `CliDriver::io_in` and
  `AcpDriver::io`, **always**, including when `user_extensions` is on, because that is exactly when
  a mod needs it. Do not set it in `launch::spawn` (that also runs the tier-1/tier-2 probes) or in
  the row's `launch.env` (that is user data and is never shown, `R-SEC-2`). The ACP adapter passes
  `process.env` through to the CLI it spawns (`acp-agent.js:5915-5925`), so the variable reaches
  Claude Code on both paths.
- **D5 — The ACP lever is `_meta.claudeCode.options` on `session/new`, and on resume and load.**
  `claude-agent-acp` 0.75.0 hardcodes `settingSources: ["user","project","local"]`, then spreads
  `_meta.claudeCode.options` over it (`acp-agent.js:5931-5936`), and merges `extraArgs`
  (`:5974-5977`). So yes, it loads user plugins today. htui sends the namespaced key from the
  transport whenever `user_extensions` is false. It does not branch on the agent's name
  (`R-AGT-5`), and ACP requires that an implementation ignore `_meta` keys it does not know, so
  `agy` is unaffected. The **content** comes from T2's probe. The candidates are
  (a) `{settings:{disableAllHooks:true}, extraArgs:{"disable-slash-commands":""}}` and
  (b) `settingSources:["project","local"]` plus `extraArgs`. The deciding probe is that the
  adapter's own **programmatic** SDK hooks (`PostToolUse`, `Stop`, `TaskCreated`, …,
  `:5979-6060`) keep firing. If `disableAllHooks` kills them, (a) is out.
- **D6 — The form.** Add a ninth Settings > Agents form field, `mods (y/n)` (label chosen by the maintainer after blueprint F6: it fits the 13-column label column), default `n`.
  It writes only `settings.user_extensions` and leaves every other settings key untouched. It
  appears for every row, because nothing branches on name or transport. The README says it only
  matters for Claude Code rows. No new table column.
- **D8 — An operator `--settings` must isolate too (added after T1 case 1a; maintainer, 2026-10-08).**
  A later `--settings` replaces htui's own rather than merging with it. So, before the spawn, an
  isolated session (`user_extensions` false) **refuses** a row whose `cli.extra_args` carry
  `--settings` (spaced or `=`-joined), unless that value is inline JSON whose object sets
  `"disableAllHooks": true`. A path value is refused, because htui does not read the operator's
  file.
  - The message names both fixes: add `"disableAllHooks":true` to that value, or set
    `mods = y`.
  - The error is `DriverError::Transport`, never `Spawn`. This is `refuse_widening`'s shape: it
    never rewrites operator arguments.
  - A row that opts back in is never checked.
  - This lands in T3.
- **D7 — Out of scope.** These stay: the managed repo's `CLAUDE.md` (ANA-29 §12 Q1; it needs
  `--safe-mode`, which the probe records only), plugin agents and plugin MCP servers (recorded,
  with a follow-up item filed if the probe shows a clean lever), and user MCP servers
  (`docs/htui-mcp.md:33-35`; no `--strict-mcp-config`).

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| argv flags | `crates/htui-agent/src/cli/mod.rs:149-232` (`argv`) | Pure function, numbered position rules in the doc comment, one comment per decision with its item tag (`MOD-91 D…`) |
| argv tests | `crates/htui-agent/tests/cli_driver.rs` `argv_is_the_ana4_line_in_order`, `persona_flags_follow_the_pairs_and_precede_extra_args`, `argv_without_mcp_is_unchanged` | Exact-list assertions. The new levers update these lists, and new tests pin the opt-in and the order before `extra_args` |
| settings field | `crates/htui-agent/src/launch.rs:283-299` `AgentSettings` | `#[serde(default)]` struct, doc comment naming the requirement |
| ACP session params | `crates/htui-agent/src/acp/mod.rs:1158-1160`; `NewSessionRequest::meta` (`agent-client-protocol-schema` 1.7.0 `v1/agent.rs:114`) | Builder chain |
| ACP param tests | `crates/htui-agent/tests/acp_driver.rs` `session_new_carries_the_mcp_server_as_stdio` (`:2024`) | Capture `session/new` params from the scripted agent, then assert on the JSON |
| live probe | `crates/htui-agent/tests/cli_live.rs` case 1 (`:948-1069`) | `#[ignore = "spends model tokens…"]`, spawn through `launch::spawn`, a throwaway cwd, pid-survival check, results printed and recorded |
| form field | `crates/htui/src/agent_settings.rs` `FIELD_LABELS`, `parse_enabled`, `draft_of`, `apply_draft`; `crates/htui/src/ui/tabs/settings/agents.rs:2056-2170, 2449-2460` | One label constant, a y/n parse through `yes_or_no`, prefill order = label order |
| errors | `DriverError::Transport` / `Spawn` (cli/mod.rs) | No new error kinds needed |
| snapshots | `crates/htui/tests/snapshots/settings__agents_{create,edit}_form.snap` | `cargo insta` full run (memory: grepping `.snap` undercounts) |

## Files to change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-agent/tests/isolation_live.rs` | CREATE | T1, T2 | `#[ignore]` live probe: the CLI matrix and the ACP lever |
| `crates/htui-agent/tests/fixtures/isolation/**` | CREATE | T1 | Two test plugins with a mod (user-scope and skills-directory), plus one skill |
| `crates/htui-agent/src/launch.rs` | UPDATE | T3 | `AgentSettings.user_extensions` |
| `crates/htui-agent/src/cli/mod.rs` | UPDATE | T3 | `argv` levers (D1/D2), `HTUI_SESSION` in `io_in` (D4), doc rule 6⅞ |
| `crates/htui-agent/tests/cli_driver.rs` | UPDATE | T3 | Exact argv lists, the opt-in, the env |
| `crates/htui-agent/src/acp/mod.rs` | UPDATE | T4 | `_meta` on new, resume and load (D5), `HTUI_SESSION` in `io` |
| `crates/htui-agent/tests/acp_driver.rs` | UPDATE | T4 | `_meta` present or absent per setting, env |
| `crates/htui/src/agent_settings.rs` | UPDATE | T5 | Ninth field, draft, merge into `settings` |
| `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | T5 | Form field, prefill |
| `crates/htui/tests/agent_settings.rs`, `crates/htui/tests/agent_settings_pg.rs` | UPDATE | T5 | The field round-trips, other settings keys are kept |
| `crates/htui/tests/snapshots/settings__agents_*form*.snap` | UPDATE | T5 | Form gains a line |
| `docs/htui-mcp.md`, `README.md` | UPDATE | T6 | Isolation section, the opt-in, `HTUI_SESSION`. Also fix the stale `--mcp-config=<json>` (it has been a file path since MOD-79) |

## Tasks

### T1 — CLI live probe (main thread; gate for T3)

- Run the probe under a **scratch `CLAUDE_CONFIG_DIR`**, never the real `~/.claude`. In the hr
  sandbox, `~/.claude` is bind-mounted read-write and shared with the host (ANA-29 §3.3), so a test
  mod installed there would load in every concurrent host session.
- Seed the scratch dir with `.credentials.json` **minus `refreshToken`**, so a token refresh can
  never rotate the host's token. The access token is valid for about 2.5 h at planning time. If it
  has expired, the maintainer refreshes it on the host.
- Seed `.claude.json` with `hasTrustDialogAccepted` for the probe checkout.
- Install a user-scope mod through a scratch directory marketplace
  (`claude plugin marketplace add` / `install`, with the scratch config dir). Place a second mod as a
  skills-directory plugin under `<checkout>/.claude/skills/<name>/`, and one plain skill. Each mod
  writes a marker through `$.process.run` / `$.ui.log`, and the probe also reads `--debug-file` for
  "hooks module … loaded".
- Matrix, with the checkout as cwd:
  - (0) no flags: both mods load (the baseline must be **positive**, or the probe proves nothing);
  - (1) `--settings '{"disableAllHooks":true}' --disable-slash-commands`;
  - (1a) the same, plus a second `--settings` from `extra_args`: does the last one win, or do they merge?
  - (2) `--safe-mode`;
  - (3) `--setting-sources project,local`;
  - (4) (1) plus `enabledPlugins:{"<p>@<m>":false}`.
- Record for each run:
  - auth (an `init` arrives, the turn succeeds);
  - htui-shaped `--mcp-config` file plus `--permission-prompt-tool`: a stub stdio MCP server
    exposing `permission_prompt` is called for a gated `Bash` call and its answer is honoured;
  - which mods loaded, and whether the skills listed in `system/init` are gone;
  - plugin agents and plugin MCP servers remaining (`system/init` `agents` and `mcp_servers`);
  - whether a settings hook (`PreToolUse` in scratch `settings.json`) fired.
- Write the results into this plan (§Probe results) and amend D1 if needed.
- **Validate**:
  `cargo test -p htui-agent --features test-support --test isolation_live -- --ignored --nocapture --test-threads=1`.

### T2 — ACP live probe (main thread; gate for T4; can run beside T1)

- Install Node ≥ 22 into `/tmp` (the sandbox has none; `mise which node` fails), and
  `claude-agent-acp@0.75.0` beside it. Under the same scratch config dir:
  - confirm the user mod loads with no `_meta`;
  - then try candidates (a) and (b) from D5 on `session/new`, and on `session/resume`/`load`;
  - for each candidate, check: the mod is absent, skills are gone, auth works, htui's MCP server
    still connects;
  - check that **the adapter's own SDK hooks still fire**: a tool call still yields its `tool_call_update`
    completion and a file edit still produces the adapter's change report.
- Pick the candidate, then amend D5.
- **Validate**: the `isolation_live` ACP case passes; the existing `acp_live` cases still pass
  with the chosen `_meta`.

### T3 — CLI default (after T1)

- **TDD**:
  - first update the exact-list argv tests;
  - add `argv_isolates_by_default`, `argv_opt_in_drops_the_levers`, and
    `the_levers_precede_extra_args`, plus a tool-less session test;
  - add an `io_in` env test for `HTUI_SESSION=1` (both settings values);
  - then implement `AgentSettings.user_extensions`, the levers in `argv`, and the env insert.
- Update `argv`'s numbered doc with a rule for the levers. Leave `refuse_widening` unchanged: an
  operator who opts back in does so through the setting, not through `extra_args`.
- **Validate**:
  `cargo test -p htui-agent --all-features --test cli_driver -- --test-threads=1`;
  `cargo clippy --workspace --all-features -- -D warnings`.

### T4 — ACP default (after T2 and T3; parallel with T5)

- **TDD** in `acp_driver.rs`:
  - `session/new` carries `_meta.claudeCode.options` with T2's content when `user_extensions` is
    false, and no `claudeCode` key when it is true;
  - the same holds on the resume and load routes;
  - `HTUI_SESSION=1` is set on the resolved env.
- Then implement this in `acp/mod.rs`.
- **Validate**: `cargo test -p htui-agent --all-features --test acp_driver -- --test-threads=1`.

### T5 — Settings > Agents opt-in (after T3; parallel with T4)

- **TDD**:
  - `user_extensions` parses y/n and round-trips through `draft_of` → form → draft;
  - `apply_draft` sets `settings.user_extensions` and keeps `cli`, `acp`, `permission`, `quota`
    and `usage` byte-for-byte;
  - `new_agent` writes it;
  - every refusal still names a form label.
- Then update the form, the prefill, and the snapshots (`cargo insta test --accept` on the full
  `htui` test run).
- **Validate**:
  `cargo test -p htui --features testkit --test agent_settings --test settings -- --test-threads=1`.
  `agent_settings_pg` runs against `HTUI_TEST_DATABASE_URL`.

### T6 — Docs and end-to-end (after T3–T5)

- **`docs/htui-mcp.md`**:
  - add "Isolation from your Claude Code setup": what is off, what stays, the opt-in, and
    `HTUI_SESSION`;
  - fix the stale `--mcp-config=<json>` line.
- **README**: under "Setting up agents" and "Claude Code without ACP", cover the new form field and
  its default.
- **End-to-end**: re-run `chat_live_cli` (`--ignored`) and `chat_live` on the production argv and
  `_meta`, with the session-start check from T1. This is the proof that the shipped default, not
  just the probe argv, keeps auth working.
- Run the full gate:
  - `cargo fmt --check`;
  - clippy with `--all-features` and featureless (`-D warnings`);
  - `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`, then grep the
    output for `SIGABRT` and `overflowed its stack`.

### Independence (file-set intersection)

| Pair | Files in common | Verdict |
|---|---|---|
| T1 ∩ T2 | `isolation_live.rs` | **Serial.** Both cases live in one file, so T2 appends after T1, or the main thread runs both |
| T3 ∩ T4 | `launch.rs` (T3 only writes it; T4 reads `user_extensions`) | **Serial**: T4 after T3 |
| T3 ∩ T5 | none in common (T5 writes the JSON key `user_extensions`, no Rust symbol from T3) | Parallel-capable, but T5 asserts T3's key name, so it runs after T3 |
| T4 ∩ T5 | none (`htui-agent/src/acp`, `htui-agent/tests/acp_driver.rs` vs `crates/htui/**`) | **Parallel** |
| T6 ∩ others | docs only | After all |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `disableAllHooks` also stops the ACP adapter's programmatic SDK hooks and breaks tool-call completion or the edit report | Medium | T2 tests this explicitly; falls back to candidate (b), `settingSources` |
| A flag breaks subscription auth, as `--bare` did (D92) | Low–medium | T1 matrix, plus T6 re-runs `chat_live_cli` on the shipped argv |
| An operator's own `--settings` in `extra_args` replaces ours and silently re-enables hooks | **Confirmed** (T1 1a: last wins) | D8: refused before the spawn unless that value itself sets `disableAllHooks` |
| The probe touches the shared `~/.claude` | Low with the scratch `CLAUDE_CONFIG_DIR` | Never write to the real dir; the access-token-only copy prevents refresh-token rotation |
| The access token expires mid-probe (about 2.5 h at planning time) | Medium | Maintainer refreshes on the host; the probe aborts with a clear auth message |
| `_meta.claudeCode.options.settings` replaces the adapter's `CLAUDE_MODEL_CONFIG`-derived settings (`:5894-5900`) | Low (env not set by htui) | Documented in the code comment; T2 notes it |
| Snapshots beyond the two form snaps move | Low | Full `cargo insta` run in T5 |
| Windows quoting of the inline JSON `--settings` | Low (ANA-4: `claude` spawns directly, not via a `.cmd` shim) | MOD-16 Windows note; fall back to a settings file in the MOD-79 private dir if a Windows run shows it |

## Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod91-gate.log
grep -E "SIGABRT|overflowed its stack" /tmp/mod91-gate.log && exit 1 || true
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Claims to verify (plan fact-check, step 3.5)

See §Verified claims.

## Verified claims

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | Every production `claude-cli` session builds its argv in `cli::argv` (engine, chat, promotion, help) | ✓ | `argv` has a single production caller, `CliDriver::io_in` (`cli/mod.rs:572`), from `CliDriver::start` (`:610-636`). Gortex text search for `"stream-json"`: production hits only in `cli/mod.rs:164,166`, and the rest are tests |
| 2 | Every ACP session opens through `AcpDriver::io` → `launch::spawn` and `session/new` at `acp/mod.rs:1158` | ✓ | `acp/mod.rs:371-389`, `:1155-1162` |
| 3 | No code passes `--bare`, `--safe-mode`, `--setting-sources`, `--disable-slash-commands` or `disableAllHooks` | ✓ | `argv` body (`cli/mod.rs:149-232`); seed `agent_claude_cli.json` `extra_args: []`; `cli_driver.rs:133-134` pins no `--bare` |
| 4 | `claude` 2.1.294 has `--settings`, `--disable-slash-commands`, `--safe-mode`, `--setting-sources`, `--debug-file`, `--plugin-dir` | ✓ | `claude --help` on this box |
| 5 | `AgentSettings` is `#[serde(default)]` over `JSONB NOT NULL DEFAULT '{}'`, so a new bool needs no migration | ✓ | `launch.rs:283-286` |
| 6 | An unparsable settings document falls back to the default (isolated) | ✓ | production `CliDriver::from_row_with_probe` (`cli/mod.rs:471`) and `AcpDriver::from_row_with_probe` (`acp/mod.rs:270`): `serde_json::from_value(…).unwrap_or_default()` |
| 7 | `claude-agent-acp` 0.75.0 loads user settings and plugins, and `_meta.claudeCode.options` overrides `settingSources` and adds `extraArgs` | ✓ | npm tarball `dist/acp-agent.js:5830-5831` (`sessionMeta`), `:5931-5936` (`settingSources:["user","project","local"]`, then `...userProvidedOptions`), `:5974-5977` (`extraArgs`) |
| 8 | The adapter re-reads `_meta` on resume and load (so htui must send it there too) | ✓ | `resumeSession`/`loadSession` → `getOrCreateSession(params)` (`:975-997`); the `createSession` param flow is to be confirmed in T2 |
| 9 | The ACP Rust crate can set `_meta` on `session/new` | ✓ | `agent-client-protocol-schema` 1.7.0 `v1/agent.rs:114` `pub fn meta(…)`; workspace pins `agent-client-protocol = "=2.1.0"` |
| 10 | The adapter passes `process.env` through, so `HTUI_SESSION` reaches the CLI it spawns | ✓ | `acp-agent.js:5915-5925` (`env = {...process.env, …}`) |
| 11 | The Settings > Agents form has 8 fields and never edits `settings` today | ✓ | `agent_settings.rs:28-37` (`FIELD_LABELS`), `:55-56` doc ("everything but `name`, `settings` …") |
| 12 | `local`/`shared_serialized` sessions run in the checkout; `worktree`/`copy` run in a scratch session dir | ✓ | `isolate/real.rs:690-701`, `:725-728`, `:860-863` |
| 13 | User MCP servers are kept on purpose | ✓ | `docs/htui-mcp.md:33-35` |
| 14 | `docs/htui-mcp.md` still says `--mcp-config=<json>` | ✓ (stale) | `docs/htui-mcp.md:33`; argv passes a file path since MOD-79 (`cli/mod.rs:190-192`) |
| 15 | htui does not depend on Claude Code skills or slash commands in a session (so `--disable-slash-commands` costs nothing) | ✓ | `R-ID-5` (`docs/REQUIREMENTS.md:75-77`: skills inlined into the prompt); MOD-9 M2 inlines bound skills; Gortex text search for `"/compact`: 0 hits |
| 16 | Subscription OAuth credentials exist here, so the live probe can run in the sandbox | ✓ | `~/.claude/.credentials.json` `claudeAiOauth` (`subscriptionType` present), access token valid about 2.5 h at 22:40 |
| 17 | The sandbox has no `node` | ✓ | `which node` empty; `mise which node` errors |
| 18 | `~/.claude` is shared read-write with the host from the hr sandbox | ✓ | ANA-29 §3.3 (`docker/hr/compose.hr.yaml:99-114`) |
| 19 | Only `cli_driver.rs` and `cli_live.rs` assert the argv's fixed flags | ✓ | `grep -rl '"--include-partial-messages"' crates`: `cli/mod.rs`, `tests/cli_driver.rs`, `tests/cli_live.rs` |
| 20 | `acp_driver.rs` already captures `session/new` params for assertions | ✓ | `session_new_carries_the_mcp_server_as_stdio` (`acp_driver.rs:2024-2040`) |
| 21 | T4 and T5 touch disjoint files | ✓ | §Independence |
| 22 | `disableAllHooks` stops installed and skills-directory mods | ✓ (T1) | §Probe results T1 case 1 (`--plugin-dir` not probed: htui never passes it) |
| 23 | `disableAllHooks` leaves the adapter's programmatic SDK hooks running | ✓ (T2) | §Probe results T2 (a): `current_mode_update: plan` still arrives |

## Probe results

### T1 — CLI (2026-10-08, `claude` 2.1.294, subscription OAuth, scratch `CLAUDE_CONFIG_DIR`)

Setup: a user-scope mod `probe-user@mod91-probe` (directory marketplace, `claude plugin install`,
scope user), a skills-directory mod `probe-proj@skills-dir` in a trusted git checkout used as cwd,
one user skill, one project skill, and a user `SessionStart` settings hook that touches a marker.
htui-shaped spawn: `-p` stream-json, `--mcp-config=<file>` naming a stub stdio server `htui` that
serves `permission_prompt` (always allows, logs each call), `--permission-prompt-tool
mcp__htui__permission_prompt`, `--permission-mode default`, one turn asking for a gated
`Bash touch`. Mods read from `--debug-file` ("hooks module <id> loaded" / "not loaded").

| Case | Flags | Auth | htui MCP + prompt tool | User mod | Skills-dir mod | Settings hook | Skills | Verdict |
|---|---|---|---|---|---|---|---|---|
| 0 | none | ✓ | connected, 1 prompt call, Bash ran | **loaded** | **loaded** | **fired** | both listed | baseline positive: the hazard is real |
| 1 | `--settings '{"disableAllHooks":true}' --disable-slash-commands` | ✓ | connected, 1 prompt call, Bash ran | not loaded | not loaded | not fired | 0 | **D1 passes** |
| 1a | 1 then `--settings '{"env":{…}}'` (an `extra_args` copy) | ✓ | ✓ | **loaded** | **loaded** | **fired** | 0 | **a later `--settings` replaces ours, no merge** |
| 2 | `--safe-mode` | — | **exit 1**: "MCP tool mcp__htui__permission_prompt (passed via --permission-prompt-tool) not found" | — | — | — | — | out: drops htui's server |
| 3 | `--setting-sources project,local` | ✓ | ✓ | not loaded | **loaded** | not fired | project skill listed | not enough alone |
| 4 | 1 plus `enabledPlugins:{<each>:false}` | ✓ | ✓ | gone | gone | not fired | 0 | removes whole plugins, but only by **name**: not a generic default |
| 5 | 1, user plugin also ships an agent and an `.mcp.json` server | ✓ | ✓ | not loaded | not loaded | not fired | 0 | plugin agent `probe-user:probe-agent` **stays**; plugin MCP server `plugin:probe-user:probe-mcp` **stays, connected** |

Notes:
- Claude Code's **built-in** mods (`cc-plugin-agents-md@builtin`) load under every lever:
  "built-in plugins load regardless". They are not the user's and are out of MOD-91's scope.
- Plugin agents and plugin MCP servers survive `disableAllHooks` (case 5). The only lever that
  removes them is per-name `enabledPlugins` (case 4). A generic default would have to enumerate the
  user's installed plugins, so per D7 no follow-up lever is adopted, and none is filed.
- **Case 1a amends the plan's risk row.** An operator `--settings` in `extra_args` silently undoes
  isolation. See D8.

### T2 — ACP (2026-10-08, `claude-agent-acp` 0.75.0 on Node 22.23.3 in `/tmp`, same scratch config and mods)

The client is a JSON-RPC stdio driver: `initialize`, then `session/new` with htui's stub server as a
stdio `mcpServers` entry, then one prompt ("call EnterPlanMode, then reply DONE"). Every permission
request is allowed. `--debug-file` travels in `_meta.claudeCode.options.extraArgs`.

| Case | `_meta.claudeCode.options` | Turn | htui MCP | Mods | Settings hook | Commands/skills | Adapter SDK hook (`PostToolUse` → `current_mode_update: plan`) |
|---|---|---|---|---|---|---|---|
| baseline | debug-file only | `end_turn` | connected | **both loaded** | **fired** | 73 commands incl. `probe-uskill`, `probe-skill` | fired |
| (a) | `{settings:{disableAllHooks:true}, extraArgs:{"disable-slash-commands":""}}` | `end_turn` | connected | **both not loaded** | not fired | **0** | **still fires** |

- **D5 resolves to candidate (a).** `disableAllHooks` stops settings hooks and plugin mods but not
  the adapter's programmatic SDK callbacks, so (b) (`settingSources`) is not needed.
- `session/resume` and `session/load` reach `createSession` through `getOrCreateSession`, which
  forwards `params._meta` (`acp-agent.js:5699-5720`). htui must send the same `_meta` on those
  routes, as T4 planned (claim 8 now ✓).
- Over ACP, `--disable-slash-commands` empties the whole `available_commands_update` list, built-in
  commands included. htui sends no slash command (claim 15), so nothing is lost.
- `agy`'s handling of the `claudeCode` `_meta` key was not checked live, because `agy` is not
  installed in the sandbox (blueprint F14).

### Amendment: where `isolation_live.rs` lands

T1 and T2 ran as an exploratory shell and Python harness under `/tmp/mod91`, because a Rust test
written before T3/T4 would pin the probe's argv rather than the shipped one. The committed
`#[ignore]` test `crates/htui-agent/tests/isolation_live.rs` moves to **T6**. It drives the shipped
levers: `cli::argv` with the real `AgentSettings`, and the ACP driver's `_meta`. It uses the same
scratch-config method (credentials without `refreshToken`, a scratch marketplace, a skills-directory
plugin), the same stub `permission_prompt` server, and asserts this section's rows 0, 1 and 1a
(refused by D8) and T2's (a).

## Acceptance

- [ ] T1/T2 results recorded; D1/D5 amended where falsified
- [ ] Every `claude-cli` and ACP session htui starts is isolated by default; the opt-in restores today's behaviour
- [ ] `HTUI_SESSION=1` is in every agent's environment
- [ ] The Settings > Agents field round-trips without touching other settings keys
- [ ] `chat_live_cli`/`chat_live` pass on the shipped levers
- [ ] Full gate green (serial), validator green
